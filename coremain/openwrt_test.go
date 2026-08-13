package coremain

import (
	"errors"
	"io/fs"
	"os"
	"testing"
)

func TestHasOpenWrtRelease(t *testing.T) {
	if !hasOpenWrtRelease(func(string) (os.FileInfo, error) { return nil, nil }) {
		t.Fatal("existing OpenWrt release file was not detected")
	}
	if hasOpenWrtRelease(func(string) (os.FileInfo, error) { return nil, fs.ErrNotExist }) {
		t.Fatal("missing OpenWrt release file was detected")
	}
	if hasOpenWrtRelease(func(string) (os.FileInfo, error) { return nil, errors.New("permission denied") }) {
		t.Fatal("unreadable OpenWrt release file was detected")
	}
}
