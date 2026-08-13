package cache

import (
	"bytes"
	"compress/gzip"
	"encoding/hex"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	pkgcache "github.com/IrineSistiana/mosdns/v5/pkg/cache"
	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
	"github.com/miekg/dns"
	"github.com/prometheus/client_golang/prometheus"
	"gopkg.in/yaml.v3"
)

func TestCacheContractArgsYAML(t *testing.T) {
	tests := []struct {
		name string
		yaml string
		want []string
	}{
		{name: "scalar", yaml: "exclude_ip: 192.0.2.0/24 2001:db8::/32\n", want: []string{"192.0.2.0/24", "2001:db8::/32"}},
		{name: "sequence", yaml: "exclude_ip:\n  - 192.0.2.0/24\n  - 2001:db8::/32\n", want: []string{"192.0.2.0/24", "2001:db8::/32"}},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			var got Args
			if err := yaml.Unmarshal([]byte(tt.yaml), &got); err != nil {
				t.Fatal(err)
			}
			if strings.Join(got.ExcludeIPs, ",") != strings.Join(tt.want, ",") {
				t.Fatalf("exclude_ip mismatch: got %v, want %v", got.ExcludeIPs, tt.want)
			}
		})
	}
}

func TestCacheContractKeyEncoding(t *testing.T) {
	q := new(dns.Msg)
	q.SetQuestion("example.org.", dns.TypeAAAA)
	q.AuthenticatedData = true
	q.CheckingDisabled = true
	q.SetEdns0(1232, true)
	qCtx := query_context.NewContext(q)
	qCtx.QOpt().SetDo()

	ecs := &dns.EDNS0_SUBNET{
		Code:          dns.EDNS0SUBNET,
		Family:        1,
		SourceNetmask: 24,
		SourceScope:   0,
		Address:       []byte{192, 0, 2, 0},
	}
	qCtx.QOpt().Option = append(qCtx.QOpt().Option, ecs)

	got, pooled := getMsgKeyBytes(qCtx.Q(), qCtx, true)
	if pooled == nil {
		t.Fatal("getMsgKeyBytes returned no pooled buffer")
	}
	defer keyBufferPool.Put(pooled)

	want := append([]byte{adBit | cdBit | doBit, 0, byte(dns.TypeAAAA), byte(len("example.org."))}, []byte("example.org.")...)
	ecsText := ecs.String()
	want = append(want, byte(len(ecsText)))
	want = append(want, ecsText...)
	if !bytes.Equal(got, want) {
		t.Fatalf("cache key mismatch\n got: %s\nwant: %s", hex.EncodeToString(got), hex.EncodeToString(want))
	}
	if gotText, wantText := keyToString(key(string(got))), "example.org. AAAA IN [flags:AD,CD,DO] [ecs:"+ecsText+"]"; gotText != wantText {
		t.Fatalf("human-readable key mismatch: got %q, want %q", gotText, wantText)
	}
}

func TestCacheContractRejectsNonStandardQueries(t *testing.T) {
	tests := map[string]*dns.Msg{
		"response":         {MsgHdr: dns.MsgHdr{Response: true}, Question: []dns.Question{{Name: "example.org.", Qtype: dns.TypeA, Qclass: dns.ClassINET}}},
		"non-query opcode": {MsgHdr: dns.MsgHdr{Opcode: dns.OpcodeUpdate}, Question: []dns.Question{{Name: "example.org.", Qtype: dns.TypeA, Qclass: dns.ClassINET}}},
		"no question":      {},
		"two questions": {Question: []dns.Question{
			{Name: "example.org.", Qtype: dns.TypeA, Qclass: dns.ClassINET},
			{Name: "example.net.", Qtype: dns.TypeA, Qclass: dns.ClassINET},
		}},
	}

	for name, q := range tests {
		t.Run(name, func(t *testing.T) {
			qCtx := query_context.NewContext(q)
			got, pooled := getMsgKeyBytes(q, qCtx, false)
			if got != nil || pooled != nil {
				if pooled != nil {
					keyBufferPool.Put(pooled)
				}
				t.Fatalf("invalid query produced cache key %x", got)
			}
		})
	}
}

func TestCacheContractResponseTTLsAndMetadata(t *testing.T) {
	tests := []struct {
		name         string
		rcode        int
		answers      []dns.RR
		lazyTTL      int
		wantMsgTTL   time.Duration
		wantCacheTTL time.Duration
	}{
		{name: "positive", rcode: dns.RcodeSuccess, answers: []dns.RR{mustRR(t, "example.org. 60 IN A 192.0.2.1")}, wantMsgTTL: 60 * time.Second, wantCacheTTL: 60 * time.Second},
		{name: "positive lazy", rcode: dns.RcodeSuccess, answers: []dns.RR{mustRR(t, "example.org. 60 IN A 192.0.2.1")}, lazyTTL: 600, wantMsgTTL: 60 * time.Second, wantCacheTTL: 600 * time.Second},
		{name: "nxdomain", rcode: dns.RcodeNameError, wantMsgTTL: 30 * time.Second, wantCacheTTL: 30 * time.Second},
		{name: "servfail", rcode: dns.RcodeServerFailure, wantMsgTTL: 5 * time.Second, wantCacheTTL: 5 * time.Second},
		{name: "empty answer minimum", rcode: dns.RcodeSuccess, wantMsgTTL: 5 * time.Second, wantCacheTTL: 5 * time.Second},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			backend := pkgcache.New[key, *item](pkgcache.Opts{Size: 16})
			t.Cleanup(func() { _ = backend.Close() })
			q := new(dns.Msg)
			q.SetQuestion("example.org.", dns.TypeA)
			qCtx := query_context.NewContext(q)
			resp := new(dns.Msg)
			resp.SetReply(q)
			resp.Rcode = tt.rcode
			resp.Answer = tt.answers
			resp.SetEdns0(1232, true)
			qCtx.SetResponse(resp)
			qCtx.StoreValue(query_context.KeyDomainSet, "contract-set")

			before := time.Now()
			if !saveRespToCache("contract-key", qCtx, backend, tt.lazyTTL) {
				t.Fatal("response was not cached")
			}
			stored, cacheExpiry, ok := backend.Get(key("contract-key"))
			if !ok || stored == nil {
				t.Fatal("cached response is missing")
			}
			assertDurationNear(t, stored.expirationTime.Sub(before), tt.wantMsgTTL)
			assertDurationNear(t, cacheExpiry.Sub(before), tt.wantCacheTTL)
			if stored.domainSet != "contract-set" {
				t.Fatalf("domain_set mismatch: got %q", stored.domainSet)
			}
			unpacked := new(dns.Msg)
			if err := unpacked.Unpack(stored.resp); err != nil {
				t.Fatal(err)
			}
			if unpacked.IsEdns0() != nil {
				t.Fatal("cached response retained EDNS OPT")
			}
		})
	}
}

func TestCacheContractLazyHit(t *testing.T) {
	backend := pkgcache.New[key, *item](pkgcache.Opts{Size: 16})
	t.Cleanup(func() { _ = backend.Close() })
	resp := new(dns.Msg)
	resp.SetQuestion("example.org.", dns.TypeA)
	resp.Response = true
	resp.Answer = []dns.RR{mustRR(t, "example.org. 60 IN A 192.0.2.1")}
	packed, err := resp.Pack()
	if err != nil {
		t.Fatal(err)
	}
	now := time.Now()
	backend.Store(key("lazy-key"), &item{
		resp:           packed,
		storedTime:     now.Add(-time.Minute),
		expirationTime: now.Add(-time.Second),
		domainSet:      "lazy-set",
	}, now.Add(time.Minute))

	got, lazy, domainSet := getRespFromCache("lazy-key", backend, true, expiredMsgTtl)
	if got == nil || !lazy || domainSet != "lazy-set" {
		t.Fatalf("lazy lookup mismatch: response=%v lazy=%v domain_set=%q", got != nil, lazy, domainSet)
	}
	if ttl := got.Answer[0].Header().Ttl; ttl != expiredMsgTtl {
		t.Fatalf("lazy TTL mismatch: got %d, want %d", ttl, expiredMsgTtl)
	}
}

func TestCacheContractSkipsTruncatedAndExcludedResponses(t *testing.T) {
	q := new(dns.Msg)
	q.SetQuestion("example.org.", dns.TypeA)

	t.Run("truncated", func(t *testing.T) {
		backend := pkgcache.New[key, *item](pkgcache.Opts{Size: 16})
		t.Cleanup(func() { _ = backend.Close() })
		qCtx := query_context.NewContext(q.Copy())
		resp := new(dns.Msg)
		resp.SetReply(q)
		resp.Truncated = true
		resp.Answer = []dns.RR{mustRR(t, "example.org. 60 IN A 192.0.2.1")}
		qCtx.SetResponse(resp)
		if saveRespToCache("truncated", qCtx, backend, 0) {
			t.Fatal("truncated response was cached")
		}
	})

	tests := []struct {
		name    string
		cidr    string
		answer  string
		exclude bool
	}{
		{name: "excluded IPv4", cidr: "192.0.2.0/24", answer: "example.org. 60 IN A 192.0.2.8", exclude: true},
		{name: "allowed IPv4", cidr: "198.51.100.0/24", answer: "example.org. 60 IN A 192.0.2.8"},
		{name: "excluded IPv6", cidr: "2001:db8::/32", answer: "example.org. 60 IN AAAA 2001:db8::8", exclude: true},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			c := NewCache(&Args{Size: 16, ExcludeIPs: []string{tt.cidr}}, Opts{})
			t.Cleanup(func() { _ = c.Close() })
			resp := new(dns.Msg)
			resp.SetReply(q)
			resp.Answer = []dns.RR{mustRR(t, tt.answer)}
			if got := c.containsExcluded(resp); got != tt.exclude {
				t.Fatalf("containsExcluded=%v, want %v", got, tt.exclude)
			}
		})
	}
}

func TestCacheContractDumpRoundTripPreservesHeaderAndMetadata(t *testing.T) {
	source := NewCache(&Args{Size: 16}, Opts{})
	t.Cleanup(func() { _ = source.Close() })
	resp := new(dns.Msg)
	resp.SetQuestion("example.org.", dns.TypeA)
	resp.Response = true
	resp.Answer = []dns.RR{mustRR(t, "example.org. 60 IN A 192.0.2.1")}
	packed, err := resp.Pack()
	if err != nil {
		t.Fatal(err)
	}
	now := time.Now()
	source.backend.Store(key("dump-key"), &item{
		resp:           packed,
		storedTime:     now,
		expirationTime: now.Add(time.Minute),
		domainSet:      "dump-set",
	}, now.Add(time.Minute))

	var payload bytes.Buffer
	if entries, err := source.writeDump(&payload); err != nil || entries != 1 {
		t.Fatalf("writeDump entries=%d err=%v", entries, err)
	}
	gr, err := gzipReader(bytes.NewReader(payload.Bytes()))
	if err != nil {
		t.Fatal(err)
	}
	if gr.Name != dumpHeader {
		t.Fatalf("dump header=%q, want %q", gr.Name, dumpHeader)
	}
	_ = gr.Close()

	target := NewCache(&Args{Size: 16}, Opts{})
	t.Cleanup(func() { _ = target.Close() })
	if entries, err := target.readDump(bytes.NewReader(payload.Bytes())); err != nil || entries != 1 {
		t.Fatalf("readDump entries=%d err=%v", entries, err)
	}
	got, _, ok := target.backend.Get(key("dump-key"))
	if !ok || got == nil || got.domainSet != "dump-set" || !bytes.Equal(got.resp, packed) {
		t.Fatalf("dump round-trip mismatch: present=%v item=%+v", ok, got)
	}
}

func TestCacheContractMetricNamesAndLabels(t *testing.T) {
	c := NewCache(&Args{Size: 16}, Opts{MetricsTag: "contract"})
	t.Cleanup(func() { _ = c.Close() })
	registry := prometheus.NewRegistry()
	if err := c.RegMetricsTo(prometheus.WrapRegistererWithPrefix(PluginType+"_", registry)); err != nil {
		t.Fatal(err)
	}
	families, err := registry.Gather()
	if err != nil {
		t.Fatal(err)
	}
	want := map[string]bool{
		"cache_query_total":    false,
		"cache_hit_total":      false,
		"cache_lazy_hit_total": false,
		"cache_size_current":   false,
	}
	for _, family := range families {
		name := family.GetName()
		if _, ok := want[name]; !ok {
			continue
		}
		want[name] = true
		if len(family.Metric) != 1 || len(family.Metric[0].Label) != 1 || family.Metric[0].Label[0].GetName() != "tag" || family.Metric[0].Label[0].GetValue() != "contract" {
			t.Fatalf("metric %s label contract changed: %+v", name, family.Metric)
		}
	}
	for name, found := range want {
		if !found {
			t.Errorf("metric %s was not registered", name)
		}
	}
}

func TestCacheContractAPIPaths(t *testing.T) {
	c := NewCache(&Args{Size: 16}, Opts{})
	t.Cleanup(func() { _ = c.Close() })

	tests := []struct {
		method string
		path   string
		status int
		body   string
	}{
		{method: http.MethodGet, path: "/flush", status: http.StatusOK, body: "Cache flushed"},
		{method: http.MethodGet, path: "/dump", status: http.StatusOK},
		{method: http.MethodGet, path: "/save", status: http.StatusBadRequest, body: "dump_file is not configured"},
		{method: http.MethodPost, path: "/load_dump", status: http.StatusBadRequest},
		{method: http.MethodGet, path: "/show", status: http.StatusOK},
	}

	for _, tt := range tests {
		t.Run(tt.method+" "+tt.path, func(t *testing.T) {
			req := httptest.NewRequest(tt.method, tt.path, strings.NewReader("not-a-dump"))
			rec := httptest.NewRecorder()
			c.Api().ServeHTTP(rec, req)
			if rec.Code != tt.status {
				t.Fatalf("status mismatch: got %d, want %d; body=%q", rec.Code, tt.status, rec.Body.String())
			}
			if tt.body != "" && !strings.Contains(rec.Body.String(), tt.body) {
				t.Fatalf("body %q does not contain %q", rec.Body.String(), tt.body)
			}
		})
	}
}

func mustRR(t *testing.T, text string) dns.RR {
	t.Helper()
	rr, err := dns.NewRR(text)
	if err != nil {
		t.Fatal(err)
	}
	return rr
}

func assertDurationNear(t *testing.T, got, want time.Duration) {
	t.Helper()
	if got < want-time.Second || got > want+time.Second {
		t.Fatalf("duration mismatch: got %s, want near %s", got, want)
	}
}

func gzipReader(r io.Reader) (*gzip.Reader, error) {
	return gzip.NewReader(r)
}
