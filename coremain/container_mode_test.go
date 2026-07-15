package coremain

import "testing"

func TestContainerModeEnabled(t *testing.T) {
	t.Setenv(containerModeEnv, "")
	if containerModeEnabled() {
		t.Fatal("containerModeEnabled() = true, want false for empty env")
	}

	for _, value := range []string{"1", "true", "TRUE", "yes", "on"} {
		t.Setenv(containerModeEnv, value)
		if !containerModeEnabled() {
			t.Fatalf("containerModeEnabled() = false, want true for %q", value)
		}
	}

	for _, value := range []string{"0", "false", "no", "off", "docker"} {
		t.Setenv(containerModeEnv, value)
		if containerModeEnabled() {
			t.Fatalf("containerModeEnabled() = true, want false for %q", value)
		}
	}
}

func TestApplyContainerModeToUpdateStatus(t *testing.T) {
	status := UpdateStatus{
		DownloadURL:     "https://example.com/mosdns.tar.gz",
		UpdateAvailable: true,
	}

	t.Setenv(containerModeEnv, "1")
	applyContainerModeToUpdateStatus(&status)

	if status.ApplySupported {
		t.Fatal("ApplySupported = true, want false in container mode")
	}
	if status.DownloadURL != "" {
		t.Fatalf("DownloadURL = %q, want empty", status.DownloadURL)
	}
	if status.Message != containerUpdateMessage {
		t.Fatalf("Message = %q, want %q", status.Message, containerUpdateMessage)
	}
	if !status.UpdateAvailable {
		t.Fatal("UpdateAvailable should be preserved")
	}
}

func TestContainerNetworkMode(t *testing.T) {
	t.Setenv(containerModeEnv, "")
	t.Setenv(containerNetworkModeEnv, containerNetworkModeHost)
	if got := containerNetworkMode(); got != "" {
		t.Fatalf("containerNetworkMode() = %q, want empty when container mode is disabled", got)
	}

	t.Setenv(containerModeEnv, "1")
	t.Setenv(containerNetworkModeEnv, "")
	if got := containerNetworkMode(); got != containerNetworkModeBridge {
		t.Fatalf("containerNetworkMode() = %q, want %q by default", got, containerNetworkModeBridge)
	}

	t.Setenv(containerNetworkModeEnv, "HOST")
	if got := containerNetworkMode(); got != containerNetworkModeHost {
		t.Fatalf("containerNetworkMode() = %q, want %q", got, containerNetworkModeHost)
	}
}

func TestContainerAutoInitSettings(t *testing.T) {
	t.Setenv(containerAutoInitEnv, "")
	if containerAutoInitEnabled() {
		t.Fatal("containerAutoInitEnabled() = true, want false for empty env")
	}

	t.Setenv(containerAutoInitEnv, "1")
	if !containerAutoInitEnabled() {
		t.Fatal("containerAutoInitEnabled() = false, want true for 1")
	}

	t.Setenv(containerConfigInitURLEnv, "")
	if got := containerConfigInitURL(); got != defaultContainerConfigInitURL {
		t.Fatalf("containerConfigInitURL() = %q, want %q", got, defaultContainerConfigInitURL)
	}
	if got := containerConfigInitURLs(); len(got) != 3 || got[0] != defaultContainerConfigInitURL || got[1] != fallbackContainerConfigCDNURL || got[2] != fallbackContainerConfigProxyURL {
		t.Fatalf("containerConfigInitURLs() = %v, want [%q %q %q]", got, defaultContainerConfigInitURL, fallbackContainerConfigCDNURL, fallbackContainerConfigProxyURL)
	}

	t.Setenv(containerConfigInitURLEnv, " "+legacyContainerConfigInitURL+" ")
	if got := containerConfigInitURL(); got != defaultContainerConfigInitURL {
		t.Fatalf("containerConfigInitURL() = %q, want normalized built-in raw url", got)
	}
	if got := containerConfigInitURLs(); len(got) != 3 || got[0] != defaultContainerConfigInitURL || got[1] != fallbackContainerConfigCDNURL || got[2] != fallbackContainerConfigProxyURL {
		t.Fatalf("containerConfigInitURLs() = %v, want built-in fallback chain for legacy url", got)
	}

	t.Setenv(containerConfigInitURLEnv, " https://example.com/config_all.zip ")
	if got := containerConfigInitURL(); got != "https://example.com/config_all.zip" {
		t.Fatalf("containerConfigInitURL() = %q, want trimmed custom url", got)
	}
	if got := containerConfigInitURLs(); len(got) != 1 || got[0] != "https://example.com/config_all.zip" {
		t.Fatalf("containerConfigInitURLs() = %v, want single trimmed custom url", got)
	}
}

func TestWebUIPortChangeSupported(t *testing.T) {
	t.Setenv(containerModeEnv, "")
	if !webUIPortChangeSupported() {
		t.Fatal("webUIPortChangeSupported() = false, want true outside container mode")
	}

	t.Setenv(containerModeEnv, "1")
	t.Setenv(containerNetworkModeEnv, containerNetworkModeBridge)
	if webUIPortChangeSupported() {
		t.Fatal("webUIPortChangeSupported() = true, want false in bridge container mode")
	}

	t.Setenv(containerNetworkModeEnv, containerNetworkModeHost)
	if !webUIPortChangeSupported() {
		t.Fatal("webUIPortChangeSupported() = false, want true in host container mode")
	}
}

func TestSpecialGroupPortMappingMessage(t *testing.T) {
	t.Setenv(containerModeEnv, "1")
	t.Setenv(containerNetworkModeEnv, containerNetworkModeBridge)
	if got := specialGroupPortMappingMessage(6053); got == "" {
		t.Fatal("specialGroupPortMappingMessage() = empty, want bridge mode hint")
	}

	t.Setenv(containerNetworkModeEnv, containerNetworkModeHost)
	if got := specialGroupPortMappingMessage(6053); got != "" {
		t.Fatalf("specialGroupPortMappingMessage() = %q, want empty in host mode", got)
	}
}
