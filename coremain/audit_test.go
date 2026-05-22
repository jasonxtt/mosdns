package coremain

import "testing"

func TestComputeEffectiveTagDirectCandidatePromotedToProxy(t *testing.T) {
	got := computeEffectiveTag("订阅直连", "nocnfake", "", "sequence_fakeip_addlist")
	if got != "直连候选转代理" {
		t.Fatalf("expected direct-candidate correction label, got %q", got)
	}
}

func TestComputeEffectiveTagKeepsNoVTagOnCorrection(t *testing.T) {
	got := computeEffectiveTag("记忆无V6|订阅直连", "nocnfake", "", "sequence_fakeip_addlist")
	if got != "记忆无V6|直连候选转代理" {
		t.Fatalf("expected no-v6 correction label, got %q", got)
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

func TestComputeEffectiveTagDirectCandidatePromotedToProxyExitVariant(t *testing.T) {
	got := computeEffectiveTag("订阅直连", "nocnfake", "", "sequence_fakeip_addlist_exit")
	if got != "直连候选转代理" {
		t.Fatalf("expected direct-candidate correction label for exit variant, got %q", got)
	}
}

func TestComputeEffectiveTagMemoryCorrectionExitVariant(t *testing.T) {
	got := computeEffectiveTag("记忆直连", "nocnfake", "", "sequence_fakeip_addlist_exit")
	if got != "记忆直连转代理" {
		t.Fatalf("expected memory correction label for exit variant, got %q", got)
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
