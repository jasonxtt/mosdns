//go:build !linux || !cgo || !mosdns_rust

package rust_bridge

import "errors"

func newNativeABI() (queryABI, error) {
	return nil, errors.New("rust query ABI unavailable in this build")
}
