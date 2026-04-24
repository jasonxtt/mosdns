package coremain

import (
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"os"
	"path/filepath"
	"strconv"
	"strings"

	"github.com/IrineSistiana/mosdns/v5/mlog"
	"go.uber.org/zap"
)

const (
	webUIPortSettingsFilename = "webui_port_settings.json"
	defaultRestartEndpoint    = "http://127.0.0.1:9099/api/v1/system/restart"
)

type webUIPortSettings struct {
	Port int `json:"port"`
}

func webUIPortSettingsPath() string {
	if strings.TrimSpace(MainConfigBaseDir) == "" {
		return webUIPortSettingsFilename
	}
	return filepath.Join(MainConfigBaseDir, webUIPortSettingsFilename)
}

func normalizeWebUIPort(port int) (int, error) {
	if port < 1 || port > 65535 {
		return 0, errors.New("端口必须在 1-65535 之间")
	}
	return port, nil
}

func parsePortFromListenAddr(addr string) (int, error) {
	target := strings.TrimSpace(addr)
	if target == "" {
		return 0, errors.New("empty listen address")
	}
	_, portText, err := net.SplitHostPort(target)
	if err != nil {
		return 0, err
	}
	port, err := strconv.Atoi(portText)
	if err != nil {
		return 0, err
	}
	return port, nil
}

func replaceListenAddrPort(baseAddr string, port int) (string, error) {
	if _, err := normalizeWebUIPort(port); err != nil {
		return "", err
	}
	target := strings.TrimSpace(baseAddr)
	if target == "" {
		return net.JoinHostPort("", strconv.Itoa(port)), nil
	}
	host, _, err := net.SplitHostPort(target)
	if err != nil {
		return "", fmt.Errorf("无效监听地址 %q: %w", target, err)
	}
	return net.JoinHostPort(host, strconv.Itoa(port)), nil
}

func loadWebUIPortSettings() (webUIPortSettings, error) {
	var settings webUIPortSettings
	raw, err := os.ReadFile(webUIPortSettingsPath())
	if err != nil {
		if os.IsNotExist(err) {
			return settings, nil
		}
		return settings, err
	}
	if err := json.Unmarshal(raw, &settings); err != nil {
		return settings, err
	}
	if settings.Port != 0 {
		if _, err := normalizeWebUIPort(settings.Port); err != nil {
			return webUIPortSettings{}, err
		}
	}
	return settings, nil
}

func saveWebUIPortSettings(port int) error {
	normalized, err := normalizeWebUIPort(port)
	if err != nil {
		return err
	}
	payload := webUIPortSettings{Port: normalized}
	data, err := json.MarshalIndent(payload, "", "  ")
	if err != nil {
		return err
	}
	return writeManagedFile(
		webUIPortSettingsPath(),
		data,
		func(raw []byte) error {
			var parsed webUIPortSettings
			if err := json.Unmarshal(raw, &parsed); err != nil {
				return err
			}
			_, err := normalizeWebUIPort(parsed.Port)
			return err
		},
		nil,
		nil,
	)
}

func applyWebUIPortOverride(cfg *Config) {
	if cfg == nil {
		return
	}
	settings, err := loadWebUIPortSettings()
	if err != nil {
		mlog.L().Warn("failed to load webui port settings", zap.Error(err))
		return
	}
	if settings.Port == 0 {
		return
	}
	nextAddr, err := replaceListenAddrPort(cfg.API.HTTP, settings.Port)
	if err != nil {
		mlog.L().Warn("invalid webui port override, ignored", zap.Int("port", settings.Port), zap.Error(err))
		return
	}
	if strings.TrimSpace(cfg.API.HTTP) != strings.TrimSpace(nextAddr) {
		mlog.L().Info("webui port override applied",
			zap.String("from", cfg.API.HTTP),
			zap.String("to", nextAddr))
	}
	cfg.API.HTTP = nextAddr
}

func activeListenAddr() string {
	if currentMosdns == nil {
		return ""
	}
	return strings.TrimSpace(currentMosdns.apiHTTPAddr)
}

func localEndpointHost(host string) string {
	h := strings.TrimSpace(host)
	switch h {
	case "", "0.0.0.0", "::":
		return "127.0.0.1"
	}
	return h
}

func resolveLocalRestartEndpoint() string {
	if endpoint := strings.TrimSpace(os.Getenv("MOSDNS_RESTART_ENDPOINT")); endpoint != "" {
		return endpoint
	}
	addr := activeListenAddr()
	if addr == "" {
		return defaultRestartEndpoint
	}
	host, portText, err := net.SplitHostPort(addr)
	if err != nil {
		return defaultRestartEndpoint
	}
	host = localEndpointHost(host)
	return "http://" + net.JoinHostPort(host, portText) + "/api/v1/system/restart"
}

func checkWebUIPortAvailable(activeAddr string, port int) error {
	if _, err := normalizeWebUIPort(port); err != nil {
		return err
	}
	base := strings.TrimSpace(activeAddr)
	if base == "" {
		base = ":9099"
	}
	targetAddr, err := replaceListenAddrPort(base, port)
	if err != nil {
		return err
	}
	if strings.TrimSpace(targetAddr) == strings.TrimSpace(activeAddr) {
		return nil
	}
	ln, err := net.Listen("tcp", targetAddr)
	if err != nil {
		return fmt.Errorf("端口 %d 已被占用", port)
	}
	_ = ln.Close()
	return nil
}
