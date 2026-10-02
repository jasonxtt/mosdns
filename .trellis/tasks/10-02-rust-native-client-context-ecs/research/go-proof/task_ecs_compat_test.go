package cache

import (
	"bytes"
	"compress/gzip"
	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
	"github.com/miekg/dns"
	"io"
	"net"
	"os"
	"path/filepath"
	"testing"
	"time"
)

func taskKey(t *testing.T, ip string, family uint16, mask uint8) []byte {
	t.Helper()
	q := new(dns.Msg)
	q.SetQuestion("a.", dns.TypeA)
	ctx := query_context.NewContext(q)
	address := net.ParseIP(ip)
	if family == 1 {
		address = address.To4()
	}
	ctx.QOpt().Option = append(ctx.QOpt().Option, &dns.EDNS0_SUBNET{Code: dns.EDNS0SUBNET, Family: family, SourceNetmask: mask, SourceScope: 0, Address: address})
	key, pool := getMsgKeyBytes(ctx.Q(), ctx, true)
	defer keyBufferPool.Put(pool)
	return append([]byte(nil), key...)
}
func taskAnswer(t *testing.T, last string) []byte {
	t.Helper()
	q := new(dns.Msg)
	q.SetQuestion("a.", dns.TypeA)
	r := new(dns.Msg)
	r.SetReply(q)
	rr, err := dns.NewRR("a. 60 IN A 192.0.2." + last)
	if err != nil {
		t.Fatal(err)
	}
	r.Answer = []dns.RR{rr}
	wire, err := r.Pack()
	if err != nil {
		t.Fatal(err)
	}
	return wire
}
func TestTaskECSGenerate(t *testing.T) {
	root := os.Getenv("TASK_ECS_PROOF")
	if root == "" {
		t.Skip("task-only controlled fixture")
	}
	var result bytes.Buffer
	writer := gzip.NewWriter(&result)
	writer.Name = dumpHeader
	cases := []struct {
		ip, last string
		family   uint16
		mask     uint8
	}{
		{"192.0.2.0", "10", 1, 24}, {"192.0.2.199", "11", 1, 24}, {"192.0.2.55", "12", 1, 24},
		{"203.0.113.99", "13", 1, 16}, {"2001:db8:1234:5678::abcd", "20", 2, 48},
		{"::ffff:192.0.2.199", "30", 2, 120},
	}
	for _, spec := range cases {
		cache := NewCache(&Args{Size: 16, EnableECS: true}, Opts{})
		keyBytes := taskKey(t, spec.ip, spec.family, spec.mask)
		cache.backend.Store(key(string(keyBytes)), &item{resp: taskAnswer(t, spec.last), storedTime: time.Unix(2000000000, 0), expirationTime: time.Unix(2000000060, 0), domainSet: "go-ecs"}, time.Unix(2000000120, 0))
		var dump bytes.Buffer
		if _, err := cache.writeDump(&dump); err != nil {
			t.Fatal(err)
		}
		reader, err := gzip.NewReader(bytes.NewReader(dump.Bytes()))
		if err != nil {
			t.Fatal(err)
		}
		block, err := io.ReadAll(reader)
		if err != nil {
			t.Fatal(err)
		}
		reader.Close()
		if _, err := writer.Write(block); err != nil {
			t.Fatal(err)
		}
		cache.Close()
	}
	if err := writer.Close(); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(root, "go-ecs-v2.gz"), result.Bytes(), 0600); err != nil {
		t.Fatal(err)
	}
}
func TestTaskECSReadNative(t *testing.T) {
	root := os.Getenv("TASK_ECS_PROOF")
	if root == "" {
		t.Skip("task-only controlled fixture")
	}
	if os.Getenv("TASK_ECS_READ_NATIVE") != "1" {
		t.Skip("native export not generated yet")
	}
	data, err := os.ReadFile(filepath.Join(root, "native-ecs-v2.gz"))
	if err != nil {
		t.Fatal(err)
	}
	cache := NewCache(&Args{Size: 16, EnableECS: true}, Opts{})
	defer cache.Close()
	count, err := cache.readDump(bytes.NewReader(data))
	if err != nil || count != 4 {
		t.Fatalf("actual Go reader: %d %v", count, err)
	}
	for _, q := range []struct {
		ip, last string
		family   uint16
		mask     uint8
	}{{"192.0.2.0", "12", 1, 24}, {"203.0.0.0", "13", 1, 16}, {"2001:db8:1234::", "20", 2, 48}, {"::ffff:192.0.2.0", "30", 2, 120}} {
		keyBytes := taskKey(t, q.ip, q.family, q.mask)
		value, expiry, ok := cache.backend.Get(key(string(keyBytes)))
		if !ok {
			t.Fatalf("canonical native export does not match Go query %s/%d", q.ip, q.mask)
		}
		msg := new(dns.Msg)
		if err := msg.Unpack(value.resp); err != nil {
			t.Fatal(err)
		}
		if len(msg.Answer) != 1 || msg.Answer[0].String() != "a.\t60\tIN\tA\t192.0.2."+q.last || value.domainSet != "go-ecs" || value.storedTime.Unix() != 2000000000 || value.expirationTime.Unix() != 2000000060 || expiry.Unix() != 2000000120 {
			t.Fatal("native export semantic mismatch")
		}
	}
	original := taskKey(t, "192.0.2.199", 1, 24)
	if _, _, ok := cache.backend.Get(key(string(original))); ok {
		t.Fatal("noncanonical prepack hostbits must require refill")
	}
}
