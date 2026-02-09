//go:build !linux

package nft_add

import (
	"github.com/IrineSistiana/mosdns/v5/coremain"
)

func init() {
	// 在非 Linux 平台上注册一个空插件，确保配置解析不报错
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
	Enable       string `yaml:"enable"`
	StartupDelay int    `yaml:"startup_delay"`
	TableFamily  string `yaml:"table_family"`
	Table        string `yaml:"table_name"`
	SetV4        string `yaml:"set_v4"`
	SetV6        string `yaml:"set_v6"`
	FixIPFile    string `yaml:"fixip"`
	NftConfFile  string `yaml:"nftfile"`

	// eBPF configurations (必须保留这些字段以适配 YAML 解析)
	EbpfEnable     string `yaml:"ebpf_enable"`
	EbpfIface      string `yaml:"ebpf_iface"`
	MihomoPort     uint16 `yaml:"mihomo_port"`
	SingboxPort    uint16 `yaml:"singbox_port"`
	MihomoFakeIPv4 string `yaml:"mihomo_fakeip_v4"`
	MihomoFakeIPv6 string `yaml:"mihomo_fakeip_v6"`
}

// 模拟 Match 方法以满足可能的接口需求
func (p *NftAdd) Match(addr any) bool {
	return false
}

type NftAdd struct{}