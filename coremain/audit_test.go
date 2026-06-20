package coremain

import (
	"runtime"
	"testing"
	"time"

	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
)

func TestComputeEffectiveTagDirectCandidatePromotedToProxy(t *testing.T) {
	got := computeEffectiveTag("订阅直连", "nocnfake", "", "sequence_fakeip")
	if got != "直连候选转代理" {
		t.Fatalf("expected direct-candidate correction label, got %q", got)
	}
}

func TestComputeEffectiveTagKeepsNoVTagOnCorrection(t *testing.T) {
	got := computeEffectiveTag("记忆无V6|订阅直连", "nocnfake", "", "sequence_fakeip")
	if got != "记忆无V6|直连候选转代理" {
		t.Fatalf("expected no-v6 correction label, got %q", got)
	}
}

func TestComputeEffectiveTagDirectCandidatePromotedToProxyExitVariant(t *testing.T) {
	got := computeEffectiveTag("订阅直连", "nocnfake", "", "sequence_fakeip_addlist_exit")
	if got != "直连候选转代理" {
		t.Fatalf("expected direct-candidate correction label for _exit variant, got %q", got)
	}
}

func TestComputeEffectiveTagMemoryCorrection(t *testing.T) {
	got := computeEffectiveTag("记忆直连", "nocnfake", "", "sequence_fakeip_addlist")
	if got != "记忆直连转代理" {
		t.Fatalf("expected memory correction label, got %q", got)
	}
}

func TestComputeEffectiveTagMemoryProxyWinsAfterLearning(t *testing.T) {
	got := computeEffectiveTag("记忆无V6|记忆代理|订阅直连", "nocnfake", "", "sequence_fakeip_addlist")
	if got != "记忆无V6|记忆代理" {
		t.Fatalf("expected learned proxy label, got %q", got)
	}
}

func TestComputeEffectiveTagKeepsForeignRealIPFilterLabel(t *testing.T) {
	got := computeEffectiveTag("!CN fakeip filter", "foreign", "", "sequence_google")
	if got != "!CN fakeip filter" {
		t.Fatalf("expected foreign real-ip filter label to remain stable, got %q", got)
	}
}

func TestAuditClientIPEqualsSupportsIPv4MappedIPv6(t *testing.T) {
	if !auditClientIPEquals("::ffff:10.0.0.10", "10.0.0.10") {
		t.Fatal("expected IPv4-mapped IPv6 and plain IPv4 to compare equal")
	}
	if auditClientIPEquals("::ffff:10.0.0.10", "10.0.0.11") {
		t.Fatal("expected different client IPs to remain different")
	}
}

func TestGetV2LogsExactSearchMatchesNormalizedClientIP(t *testing.T) {
	collector := NewAuditCollector(4)
	collector.logs = []AuditLog{
		{ClientIP: "::ffff:10.0.0.10", QueryName: "example.com", TraceID: "trace-1"},
		{ClientIP: "::ffff:10.0.0.11", QueryName: "example.net", TraceID: "trace-2"},
	}

	response := collector.GetV2Logs(V2GetLogsParams{
		Page:  1,
		Limit: 10,
		Q:     "10.0.0.10",
		Exact: true,
	})

	if got := len(response.Logs); got != 1 {
		t.Fatalf("expected one exact client IP match, got %d", got)
	}
	if response.Logs[0].ClientIP != "::ffff:10.0.0.10" {
		t.Fatalf("unexpected matched client IP %q", response.Logs[0].ClientIP)
	}
}

func TestGetV2LogsClientIPFilterSupportsMultipleAliasMatches(t *testing.T) {
	collector := NewAuditCollector(4)
	collector.logs = []AuditLog{
		{ClientIP: "::ffff:10.0.0.10", QueryName: "alpha.example", TraceID: "trace-1"},
		{ClientIP: "::ffff:10.0.0.11", QueryName: "beta.example", TraceID: "trace-2"},
		{ClientIP: "::ffff:10.0.0.12", QueryName: "gamma.example", TraceID: "trace-3"},
	}

	response := collector.GetV2Logs(V2GetLogsParams{
		Page:      1,
		Limit:     10,
		ClientIPs: []string{"10.0.0.10", "10.0.0.12"},
	})

	if got := len(response.Logs); got != 2 {
		t.Fatalf("expected two alias-backed client IP matches, got %d", got)
	}
	if response.Pagination.TotalItems != 2 {
		t.Fatalf("expected total items 2, got %d", response.Pagination.TotalItems)
	}
}

func TestGetV2LogsFiltersDomainSetAndEffectiveTagPrecisely(t *testing.T) {
	collector := NewAuditCollector(5)
	collector.logs = []AuditLog{
		{QueryName: "alpha.example", DomainSet: "special-a", EffectiveTag: "direct", TraceID: "trace-1"},
		{QueryName: "beta.example", DomainSet: "special-b", EffectiveTag: "proxy", TraceID: "trace-2"},
		{QueryName: "special-a", DomainSet: "other", EffectiveTag: "other", TraceID: "trace-3"},
	}

	domainSetResponse := collector.GetV2Logs(V2GetLogsParams{
		Page:      1,
		Limit:     10,
		DomainSet: "special-a",
	})
	if got := len(domainSetResponse.Logs); got != 1 {
		t.Fatalf("expected one domain_set match, got %d", got)
	}
	if domainSetResponse.Logs[0].TraceID != "trace-1" {
		t.Fatalf("unexpected domain_set match trace %q", domainSetResponse.Logs[0].TraceID)
	}

	effectiveResponse := collector.GetV2Logs(V2GetLogsParams{
		Page:         1,
		Limit:        10,
		EffectiveTag: "proxy",
	})
	if got := len(effectiveResponse.Logs); got != 1 {
		t.Fatalf("expected one effective_tag match, got %d", got)
	}
	if effectiveResponse.Logs[0].TraceID != "trace-2" {
		t.Fatalf("unexpected effective_tag match trace %q", effectiveResponse.Logs[0].TraceID)
	}
}

func TestClearLogsDropsBackingStoreReferences(t *testing.T) {
	collector := NewAuditCollector(4)
	collector.logs = []AuditLog{
		{
			QueryName: "alpha.example",
			Answers:   []AnswerDetail{{Type: "TXT", Data: "large-retained-answer"}},
		},
		{QueryName: "beta.example"},
		{QueryName: "gamma.example"},
		{QueryName: "delta.example"},
	}

	collector.ClearLogs()

	if len(collector.logs) != 0 {
		t.Fatalf("expected logs length 0 after clear, got %d", len(collector.logs))
	}
	if cap(collector.logs) != 0 {
		t.Fatalf("expected clear to release backing store, got capacity %d", cap(collector.logs))
	}
	if collector.head != 0 {
		t.Fatalf("expected head reset to 0, got %d", collector.head)
	}
}

func TestResetAuditContextDropsQueryContextReference(t *testing.T) {
	wrapped := &auditContext{
		Ctx:                &query_context.Context{},
		ProcessingDuration: time.Second,
	}

	resetAuditContext(wrapped)

	if wrapped.Ctx != nil {
		t.Fatal("expected reset audit context to drop query context reference")
	}
	if wrapped.ProcessingDuration != 0 {
		t.Fatalf("expected processing duration reset to 0, got %s", wrapped.ProcessingDuration)
	}
}

func TestCalculateV2WindowStatsAvoidsFullLogSnapshotAllocation(t *testing.T) {
	const logCount = 5000
	collector := NewAuditCollector(logCount)
	collector.logs = make([]AuditLog, logCount)
	collector.head = logCount / 2

	now := time.Now()
	for i := range collector.logs {
		collector.logs[i] = AuditLog{
			QueryName:  "example.test",
			QueryTime:  now.Add(-time.Duration(i%3600) * time.Second),
			DurationMs: 1,
		}
	}

	runtime.GC()
	var before runtime.MemStats
	runtime.ReadMemStats(&before)

	_ = collector.CalculateV2WindowStats()

	var after runtime.MemStats
	runtime.ReadMemStats(&after)
	if allocated := after.TotalAlloc - before.TotalAlloc; allocated > 512*1024 {
		t.Fatalf("expected window stats to avoid full log snapshot allocation, allocated %d bytes", allocated)
	}
}
