//go:build windows || !unix
// +build windows !unix

package coremain

import "os/exec"

// setProcessGroup 在不支持 Setsid 的系统上不进行操作
func setProcessGroup(cmd *exec.Cmd) {
	// 在 Windows 等系统上 Setsid 不存在，这里不做任何设置
}
