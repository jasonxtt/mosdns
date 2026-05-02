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

func TestComputeEffectiveTagKeepsForeignRealIPFilterLabel(t *testing.T) {
	got := computeEffectiveTag("!CN fakeip filter", "foreign", "", "sequence_google")
	if got != "!CN fakeip filter" {
		t.Fatalf("expected foreign real-ip filter label to remain stable, got %q", got)
	}
}
