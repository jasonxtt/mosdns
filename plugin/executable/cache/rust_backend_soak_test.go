//go:build linux && cgo && (mosdns_rust || mosdns_rust_cache)

// Reproducible cache replay/soak evidence.
//
// This test measures the Go vs Rust cache facade under a fixed synthetic
// replay workload: QPS, p50/p95/p99 wall latency, process CPU, steady/peak
// RSS, Go allocations, and cgo call counts. It is intentionally gated behind
// MOSDNS_CACHE_SOAK=1 so normal experimental CI stays fast; the exact command
// and environment are recorded in docs/rust/benchmarks/cache-foundation.md.
//
// The workload is deterministic: a fixed set of distinct cached domains with
// a fixed miss ratio, replayed cyclically by a fixed number of workers. Each
// worker reuses one query message (Exec does not mutate the query on the hit
// or miss path) so the measurement isolates cache/backend cost rather than
// per-op dns.Msg serialization. Only host load varies between runs; the
// methodology and inputs are reproducible.
package cache

import (
	"context"
	"fmt"
	"math"
	"os"
	"runtime"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
	"github.com/IrineSistiana/mosdns/v5/plugin/executable/sequence"
	"github.com/miekg/dns"
)

const (
	soakEnvEnable   = "MOSDNS_CACHE_SOAK"
	soakEnvSeconds  = "MOSDNS_CACHE_SOAK_SECS"
	soakEnvParallel = "MOSDNS_CACHE_SOAK_PARALLEL"
)

// soakDefaults keep the evidence reproducible when the env knobs are absent.
const (
	soakDefaultSeconds  = 4 * time.Second
	soakDefaultParallel = 8
	soakDistinctHits    = 256 // distinct cached domains in the replay
	soakMissRatio       = 10  // 1 in N queries is an uncached miss
	soakWarmup          = time.Second
)

// latencyHistogram is a deterministic log-linear histogram (base 2^(1/16),
// about 4.4% relative bucket width) from 1ns to 1s. It bounds memory use on
// long soaks while keeping p50/p95/p99 estimates repeatable.
type latencyHistogram struct {
	base    float64
	minNS   int64
	maxNS   int64
	buckets []uint64
}

func newLatencyHistogram() *latencyHistogram {
	base := math.Pow(2, 1.0/16.0)
	maxNS := int64(time.Second)
	n := int(math.Log(float64(maxNS))/math.Log(base)) + 2
	return &latencyHistogram{
		base:    base,
		minNS:   1,
		maxNS:   maxNS,
		buckets: make([]uint64, n),
	}
}

func (h *latencyHistogram) add(d time.Duration) {
	ns := int64(d)
	if ns < h.minNS {
		ns = h.minNS
	}
	if ns > h.maxNS {
		ns = h.maxNS
	}
	k := int(math.Log(float64(ns)/float64(h.minNS)) / math.Log(h.base))
	if k >= len(h.buckets) {
		k = len(h.buckets) - 1
	}
	h.buckets[k]++
}

func (h *latencyHistogram) merge(o *latencyHistogram) {
	for i, c := range o.buckets {
		h.buckets[i] += c
	}
}

func (h *latencyHistogram) total() uint64 {
	var n uint64
	for _, c := range h.buckets {
		n += c
	}
	return n
}

// percentile returns the bucket midpoint covering the given fraction (0..1).
func (h *latencyHistogram) percentile(p float64) time.Duration {
	total := h.total()
	if total == 0 {
		return 0
	}
	target := uint64(math.Ceil(p * float64(total)))
	var acc uint64
	for i, c := range h.buckets {
		acc += c
		if acc >= target {
			lo := float64(h.minNS) * math.Pow(h.base, float64(i))
			hi := float64(h.minNS) * math.Pow(h.base, float64(i+1))
			return time.Duration((lo + hi) / 2)
		}
	}
	return time.Duration(h.maxNS)
}

// procSelfStatCPU returns process user+system CPU in clock ticks from
// /proc/self/stat (Linux, fields 14/15 after the parenthesized comm).
func procSelfStatCPU() (float64, error) {
	data, err := os.ReadFile("/proc/self/stat")
	if err != nil {
		return 0, err
	}
	i := strings.LastIndexByte(string(data), ')')
	if i < 0 || i+1 >= len(data) {
		return 0, fmt.Errorf("unparsable /proc/self/stat")
	}
	fields := strings.Fields(string(data[i+1:]))
	if len(fields) < 13 {
		return 0, fmt.Errorf("/proc/self/stat has %d fields, want >= 13", len(fields))
	}
	// rest[0] is state (field 3); utime = rest[11], stime = rest[12].
	utime, err := strconv.ParseFloat(fields[11], 64)
	if err != nil {
		return 0, err
	}
	stime, err := strconv.ParseFloat(fields[12], 64)
	if err != nil {
		return 0, err
	}
	return utime + stime, nil
}

// rssMonitor samples VmRSS every 100ms and keeps the peak and last samples.
type rssMonitor struct {
	peak  atomic.Int64 // bytes
	last  atomic.Int64 // bytes
	stopC chan struct{}
	done  chan struct{}
}

func startRSSMonitor() *rssMonitor {
	m := &rssMonitor{stopC: make(chan struct{}), done: make(chan struct{})}
	go func() {
		defer close(m.done)
		ticker := time.NewTicker(100 * time.Millisecond)
		defer ticker.Stop()
		for {
			select {
			case <-m.stopC:
				return
			case <-ticker.C:
				kb, err := procSelfVmRSS()
				if err != nil {
					continue
				}
				b := kb * 1024
				m.last.Store(b)
				for {
					cur := m.peak.Load()
					if b <= cur || m.peak.CompareAndSwap(cur, b) {
						break
					}
				}
			}
		}
	}()
	return m
}

func (m *rssMonitor) stop() {
	close(m.stopC)
	<-m.done
}

func (m *rssMonitor) snapshot() (peak, steady int64) {
	return m.peak.Load(), m.last.Load()
}

// procSelfVmRSS returns the resident set size in kB from /proc/self/status.
func procSelfVmRSS() (int64, error) {
	data, err := os.ReadFile("/proc/self/status")
	if err != nil {
		return 0, err
	}
	for _, line := range strings.Split(string(data), "\n") {
		if strings.HasPrefix(line, "VmRSS:") {
			fields := strings.Fields(line)
			if len(fields) >= 2 {
				return strconv.ParseInt(fields[1], 10, 64)
			}
		}
	}
	return 0, fmt.Errorf("VmRSS not found in /proc/self/status")
}

func TestCacheReplaySoak(t *testing.T) {
	if os.Getenv(soakEnvEnable) != "1" {
		t.Skip("set " + soakEnvEnable + "=1 to run the reproducible cache replay/soak")
	}

	duration := soakDefaultSeconds
	if v := os.Getenv(soakEnvSeconds); v != "" {
		if secs, err := strconv.Atoi(v); err == nil && secs > 0 {
			duration = time.Duration(secs) * time.Second
		}
	}
	parallel := soakDefaultParallel
	if v := os.Getenv(soakEnvParallel); v != "" {
		if p, err := strconv.Atoi(v); err == nil && p > 0 {
			parallel = p
		}
	}
	t.Logf("soak params: duration=%s parallel=%d distinct_hits=%d miss_ratio=1/%d",
		duration, parallel, soakDistinctHits, soakMissRatio)

	for _, backend := range []string{"go", "rust"} {
		t.Run(backend, func(t *testing.T) {
			t.Setenv(rustCacheBackendEnv, backend)
			c := NewCache(&Args{Size: soakDistinctHits + 16}, Opts{})
			t.Cleanup(func() { _ = c.Close() })
			if backend == "rust" && !c.rustActive() {
				t.Skip("rust cache backend is not available in this build")
			}
			runReplaySoak(t, c, duration, parallel)
		})
	}
}

// runReplaySoak seeds a deterministic workload and drives a fixed-duration
// parallel replay, then reports QPS, latency percentiles, CPU, RSS, allocs,
// and cgo calls as structured log lines.
func runReplaySoak(t *testing.T, c *Cache, duration time.Duration, parallel int) {
	ctx := context.Background()

	// Deterministic workload: soakDistinctHits cached domains plus a fixed set
	// of uncached domains, interleaved so 1 in soakMissRatio queries misses.
	hits, misses := buildSoakWorkload()
	replay := interleaveReplay(hits, misses)

	// Seed every cached domain through the real facade path so both backends
	// hold the same snapshot before the timed window begins.
	for _, q := range hits {
		seed := query_context.NewContext(q.Copy())
		seed.SetResponse(qCtxResponse(q))
		if err := c.Exec(ctx, seed, sequence.ChainWalker{}); err != nil {
			t.Fatalf("seed %s: %v", q.Question[0].Name, err)
		}
	}

	// Warmup so allocator/cache state is steady before the timed window.
	runSoakLoop(ctx, c, replay, parallel, soakWarmup)

	runtime.GC()
	var memBefore, memAfter runtime.MemStats
	runtime.ReadMemStats(&memBefore)
	cpuBefore, err := procSelfStatCPU()
	if err != nil {
		t.Fatalf("read cpu before: %v", err)
	}
	cgoBefore := runtime.NumCgoCall()

	rssMon := startRSSMonitor()
	start := time.Now()
	hist, ops := runSoakLoop(ctx, c, replay, parallel, duration)
	elapsed := time.Since(start)
	rssMon.stop()
	peakRSS, steadyRSS := rssMon.snapshot()

	cpuAfter, err := procSelfStatCPU()
	if err != nil {
		t.Fatalf("read cpu after: %v", err)
	}
	cgoAfter := runtime.NumCgoCall()
	runtime.ReadMemStats(&memAfter)

	qps := float64(ops) / elapsed.Seconds()
	cpuSecs := (cpuAfter - cpuBefore) / 100.0 // Linux CLK_TCK = 100
	cpuPct := cpuSecs / elapsed.Seconds() * 100
	cgoDelta := cgoAfter - cgoBefore
	allocsPerOp := float64(memAfter.Mallocs-memBefore.Mallocs) / float64(ops)
	bytesPerOp := float64(memAfter.TotalAlloc-memBefore.TotalAlloc) / float64(ops)

	t.Logf("REPLAY total_ops=%d elapsed_ms=%.1f qps=%.0f", ops, elapsed.Seconds()*1000, qps)
	t.Logf("REPLAY p50_ns=%d p95_ns=%d p99_ns=%d",
		hist.percentile(0.50), hist.percentile(0.95), hist.percentile(0.99))
	t.Logf("REPLAY cpu_secs=%.3f cpu_pct=%.1f rss_peak_mib=%.1f rss_steady_mib=%.1f",
		cpuSecs, cpuPct, float64(peakRSS)/1024/1024, float64(steadyRSS)/1024/1024)
	t.Logf("REPLAY allocs_op=%.2f bytes_op=%.1f cgo_calls=%d cgo_per_op=%.2f",
		allocsPerOp, bytesPerOp, cgoDelta, float64(cgoDelta)/float64(ops))
	t.Logf("REPLAY mem_sys_mib=%.1f heap_sys_mib=%.1f heap_inuse_mib=%.1f",
		float64(memAfter.Sys)/1024/1024, float64(memAfter.HeapSys)/1024/1024, float64(memAfter.HeapInuse)/1024/1024)
}

func buildSoakWorkload() (hits, misses []*dns.Msg) {
	for i := 0; i < soakDistinctHits; i++ {
		q := new(dns.Msg)
		q.SetQuestion(fmt.Sprintf("d%d.example.", i), dns.TypeA)
		hits = append(hits, q)
	}
	// A fixed bounded miss set keeps the (empty) miss population stable so the
	// cache snapshot and behavior stay constant across the whole soak.
	for i := 0; i < soakDistinctHits/soakMissRatio; i++ {
		q := new(dns.Msg)
		q.SetQuestion(fmt.Sprintf("miss%d.example.", i), dns.TypeA)
		misses = append(misses, q)
	}
	return hits, misses
}

func interleaveReplay(hits, misses []*dns.Msg) []*dns.Msg {
	total := len(hits) + len(misses)
	replay := make([]*dns.Msg, 0, total)
	hitIdx, missIdx := 0, 0
	for i := 0; i < total; i++ {
		if i > 0 && i%soakMissRatio == 0 && missIdx < len(misses) {
			replay = append(replay, misses[missIdx])
			missIdx++
		} else {
			replay = append(replay, hits[hitIdx])
			hitIdx++
		}
	}
	return replay
}

func qCtxResponse(q *dns.Msg) *dns.Msg {
	r := new(dns.Msg)
	r.SetReply(q)
	r.Answer = []dns.RR{mustRRSoak(q.Question[0].Name, "192.0.2.1")}
	return r
}

func mustRRSoak(name, ip string) dns.RR {
	rr, err := dns.NewRR(fmt.Sprintf("%s 300 IN A %s", name, ip))
	if err != nil {
		panic(err)
	}
	return rr
}

// runSoakLoop drives `parallel` workers replaying the deterministic trace for
// the given duration and returns the merged latency histogram and total Exec
// calls. Each worker keeps its own lock-free histogram to avoid adding a
// shared mutex to the measured path; histograms merge only after the loop.
func runSoakLoop(ctx context.Context, c *Cache, replay []*dns.Msg, parallel int, duration time.Duration) (*latencyHistogram, int64) {
	// A timeout context broadcasts the stop signal to every worker; a shared
	// time.After channel would only be received by one goroutine and the rest
	// would spin forever.
	soakCtx, cancel := context.WithTimeout(ctx, duration)
	defer cancel()
	var ops atomic.Int64
	hists := make([]*latencyHistogram, parallel)
	for i := range hists {
		hists[i] = newLatencyHistogram()
	}
	var wg sync.WaitGroup
	for w := 0; w < parallel; w++ {
		wg.Add(1)
		go func(w int) {
			defer wg.Done()
			h := hists[w]
			query := replay[w%len(replay)]
			i := w
			for {
				select {
				case <-soakCtx.Done():
					return
				default:
				}
				qCtx := query_context.NewContext(query)
				start := time.Now()
				if err := c.Exec(ctx, qCtx, sequence.ChainWalker{}); err != nil {
					return
				}
				h.add(time.Since(start))
				ops.Add(1)
				i++
				query = replay[i%len(replay)]
			}
		}(w)
	}
	wg.Wait()
	merged := newLatencyHistogram()
	for _, h := range hists {
		merged.merge(h)
	}
	return merged, ops.Load()
}
