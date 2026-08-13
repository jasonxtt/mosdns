package coremain

import "os"

const openWrtReleasePath = "/etc/openwrt_release"

func configManagementEnabled() bool {
	return !hasOpenWrtRelease(os.Stat)
}

func hasOpenWrtRelease(stat func(string) (os.FileInfo, error)) bool {
	_, err := stat(openWrtReleasePath)
	return err == nil
}
