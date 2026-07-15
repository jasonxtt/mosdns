package coremain

import (
	"fmt"
	"os"
	"strings"
)

const (
	containerModeEnv                = "MOSDNS_CONTAINER_MODE"
	containerNetworkModeEnv         = "MOSDNS_CONTAINER_NETWORK_MODE"
	containerAutoInitEnv            = "MOSDNS_AUTO_INIT"
	containerConfigInitURLEnv       = "MOSDNS_CONFIG_INIT_URL"
	containerNetworkModeBridge      = "bridge"
	containerNetworkModeHost        = "host"
	defaultContainerConfigInitURL   = "https://raw.githubusercontent.com/jasonxtt/file/main/mosdns/config/config_all.zip"
	fallbackContainerConfigCDNURL   = "https://cdn.jsdelivr.net/gh/jasonxtt/file@main/mosdns/config/config_all.zip"
	fallbackContainerConfigProxyURL = "https://ghproxy.net/https://raw.githubusercontent.com/jasonxtt/file/main/mosdns/config/config_all.zip"
	containerUpdateMessage          = "容器版请拉取新镜像并重建容器，不支持在 WebUI 内直接更新二进制。"
	containerWebUIPortMessage       = "当前为 bridge 端口映射模式，不支持在 WebUI 中变更端口。请通过Compose文件或容器运行参数修改。"
	containerUpdateConflictReason   = "容器版不支持在 WebUI 内直接更新二进制，请改为拉取新镜像并重建容器。"
	containerConfigManageMessage    = "容器版请拉取新镜像并重建容器，不支持在 WebUI 内直接更新配置文件。"
	containerPortMappingMessageTpl  = "当前容器为 bridge 端口映射模式。专属分流组端口 %d 只会在容器内监听；请在 Docker Compose 或容器运行参数中新增 %d/tcp 和 %d/udp 端口映射后，外部客户端才能访问。"
)

func containerModeEnabled() bool {
	switch os.Getenv(containerModeEnv) {
	case "1", "true", "TRUE", "True", "yes", "YES", "on", "ON":
		return true
	default:
		return false
	}
}

func containerNetworkMode() string {
	if !containerModeEnabled() {
		return ""
	}

	switch strings.ToLower(strings.TrimSpace(os.Getenv(containerNetworkModeEnv))) {
	case containerNetworkModeHost:
		return containerNetworkModeHost
	default:
		return containerNetworkModeBridge
	}
}

func containerAutoInitEnabled() bool {
	switch os.Getenv(containerAutoInitEnv) {
	case "1", "true", "TRUE", "True", "yes", "YES", "on", "ON":
		return true
	default:
		return false
	}
}

func containerConfigInitURL() string {
	if s := strings.TrimSpace(os.Getenv(containerConfigInitURLEnv)); s != "" {
		return s
	}
	return defaultContainerConfigInitURL
}

func containerConfigInitURLs() []string {
	if s := strings.TrimSpace(os.Getenv(containerConfigInitURLEnv)); s != "" {
		return []string{s}
	}
	return []string{
		defaultContainerConfigInitURL,
		fallbackContainerConfigCDNURL,
		fallbackContainerConfigProxyURL,
	}
}

func webUIPortChangeSupported() bool {
	return !containerModeEnabled() || containerNetworkMode() == containerNetworkModeHost
}

func specialGroupPortMappingRequired(port int) bool {
	return port != 0 && containerModeEnabled() && containerNetworkMode() == containerNetworkModeBridge
}

func specialGroupPortMappingMessage(port int) string {
	if !specialGroupPortMappingRequired(port) {
		return ""
	}
	return fmt.Sprintf(containerPortMappingMessageTpl, port, port, port)
}

func applyContainerModeToUpdateStatus(status *UpdateStatus) {
	if status == nil {
		return
	}

	status.ApplySupported = !containerModeEnabled()
	if status.ApplySupported {
		return
	}

	status.DownloadURL = ""
	if status.Message == "" {
		status.Message = containerUpdateMessage
	}
}
