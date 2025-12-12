//go:build !windows
// +build !windows

package coremain

import (
	"os/exec"
	"syscall"
)

// setProcessGroup 在 Unix-like 系统上设置 Setsid，使新进程脱离父进程
func setProcessGroup(cmd *exec.Cmd) {
	cmd.SysProcAttr = &syscall.SysProcAttr{Setsid: true}
}
