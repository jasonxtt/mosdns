package coremain

import "os"

const (
	openWrtModeEnv              = "MOSDNS_OPENWRT_MODE"
	openWrtUpdateMessage        = "OpenWrt 版请通过 LuCI 或系统软件源升级 MosDNS-T。"
	openWrtUpdateConflictReason = "OpenWrt 版不支持在 WebUI 内直接更新程序，请通过 LuCI 或系统软件源升级 MosDNS-T。"
)

func openWrtModeEnabled() bool {
	switch os.Getenv(openWrtModeEnv) {
	case "1", "true", "TRUE", "True", "yes", "YES", "on", "ON":
		return true
	default:
		return false
	}
}

func configManagementEnabled() bool {
	return !openWrtModeEnabled()
}

func applyOpenWrtModeToUpdateStatus(status *UpdateStatus) {
	if status == nil {
		return
	}

	status.ApplySupported = !openWrtModeEnabled()
	if status.ApplySupported {
		return
	}

	status.DownloadURL = ""
	if status.Message == "" {
		status.Message = openWrtUpdateMessage
	}
}
