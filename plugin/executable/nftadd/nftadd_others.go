//go:build !linux

package nft_add

import (
	"github.com/IrineSistiana/mosdns/v5/coremain"
)

func init() {
    // Register a dummy function to prevent build errors on Windows/macOS
	coremain.RegNewPluginFunc(PluginType, func(bp *coremain.BP, args any) (any, error) {
		return nil, nil 
	}, func() any { return new(Args) })
}

const PluginType = "nft_add"

type Args struct {
	Socks5      string    `yaml:"socks5,omitempty"`
	LocalConfig string    `yaml:"local_config"`
	NftConfig   NftConfig `yaml:"nft_config"`
}

type NftConfig struct {
	Enable string `yaml:"enable"`
}
