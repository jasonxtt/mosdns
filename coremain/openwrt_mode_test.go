package coremain

import "testing"

func TestOpenWrtModeEnabled(t *testing.T) {
	t.Setenv(openWrtModeEnv, "")
	if openWrtModeEnabled() {
		t.Fatal("openWrtModeEnabled() = true, want false for empty env")
	}

	for _, value := range []string{"1", "true", "TRUE", "yes", "on"} {
		t.Setenv(openWrtModeEnv, value)
		if !openWrtModeEnabled() {
			t.Fatalf("openWrtModeEnabled() = false, want true for %q", value)
		}
	}

	for _, value := range []string{"0", "false", "no", "off", "openwrt"} {
		t.Setenv(openWrtModeEnv, value)
		if openWrtModeEnabled() {
			t.Fatalf("openWrtModeEnabled() = true, want false for %q", value)
		}
	}
}

func TestApplyOpenWrtModeToUpdateStatus(t *testing.T) {
	status := UpdateStatus{
		DownloadURL:     "https://example.com/mosdns.tar.gz",
		UpdateAvailable: true,
	}

	t.Setenv(openWrtModeEnv, "1")
	applyOpenWrtModeToUpdateStatus(&status)

	if status.ApplySupported {
		t.Fatal("ApplySupported = true, want false in OpenWrt mode")
	}
	if status.DownloadURL != "" {
		t.Fatalf("DownloadURL = %q, want empty", status.DownloadURL)
	}
	if status.Message != openWrtUpdateMessage {
		t.Fatalf("Message = %q, want %q", status.Message, openWrtUpdateMessage)
	}
	if !status.UpdateAvailable {
		t.Fatal("UpdateAvailable should be preserved")
	}
}
