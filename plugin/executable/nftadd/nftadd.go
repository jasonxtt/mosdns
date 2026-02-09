//go:build linux

/*
 * Copyright (C) 2020-2022, IrineSistiana
 * Copyright (C) 2024, Modified for nft_add requirement
 *
 * This file is part of mosdns.
 */

package nft_add

import (
	"bufio"
	"bytes"
	"compress/zlib"
	"context"
	_ "embed"
	"encoding/binary"
	"encoding/json"
	"fmt"
	"io"
	"log"
	"net"
	"net/http"
	"net/netip"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/IrineSistiana/mosdns/v5/coremain"
	"github.com/IrineSistiana/mosdns/v5/pkg/matcher/netlist"
	"github.com/IrineSistiana/mosdns/v5/plugin/data_provider"
	"github.com/cilium/ebpf"
	"github.com/cilium/ebpf/link"
	"github.com/go-chi/chi/v5"
	"github.com/sagernet/sing/common/varbin"
	"go4.org/netipx"
	"golang.org/x/net/proxy"
)

//go:embed proxy.o
var ebpfProg []byte

const (
	PluginType      = "nft_add"
	downloadTimeout = 60 * time.Second
)

func init() {
	coremain.RegNewPluginFunc(PluginType, newNftAdd, func() any { return new(Args) })
}

// IPReceiver is used for streaming IP data into different targets
type IPReceiver interface {
	AddPrefix(netip.Prefix) error
}

type builderReceiver struct {
	builder *netipx.IPSetBuilder
}

func (r *builderReceiver) AddPrefix(p netip.Prefix) error {
	r.builder.AddPrefix(p)
	return nil
}

type listReceiver struct {
	l *netlist.List
}

func (r *listReceiver) AddPrefix(p netip.Prefix) error {
	r.l.Append(p)
	return nil
}

type counterReceiver struct {
	count int
}

func (r *counterReceiver) AddPrefix(_ netip.Prefix) error {
	r.count++
	return nil
}

// Args holds the configuration
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

	EbpfEnable     string `yaml:"ebpf_enable"`
	EbpfIface      string `yaml:"ebpf_iface"`
	MihomoPort     uint16 `yaml:"mihomo_port"`
	SingboxPort    uint16 `yaml:"singbox_port"`
	MihomoFakeIPv4 string `yaml:"mihomo_fakeip_v4"`
	MihomoFakeIPv6 string `yaml:"mihomo_fakeip_v6"`
}

type RuleSource struct {
	Name                string    `json:"name"`
	Type                string    `json:"type"`
	Files               string    `json:"files"`
	URL                 string    `json:"url"`
	Enabled             bool      `json:"enabled"`
	AutoUpdate          bool      `json:"auto_update"`
	UpdateIntervalHours int       `json:"update_interval_hours"`
	RuleCount           int       `json:"rule_count"`
	LastUpdated         time.Time `json:"last_updated"`
}

type NftAdd struct {
	matcher atomic.Value
	mu      sync.RWMutex
	sources map[string]*RuleSource
	nftArgs NftConfig

	localConfigFile string
	httpClient      *http.Client
	ctx             context.Context
	cancel          context.CancelFunc

	startupDone atomic.Bool
	ebpfLink    link.Link
	ebpfMap     *ebpf.Map
}

var _ data_provider.IPMatcherProvider = (*NftAdd)(nil)
var _ io.Closer = (*NftAdd)(nil)
var _ netlist.Matcher = (*NftAdd)(nil)

func newNftAdd(bp *coremain.BP, args any) (any, error) {
	cfg := args.(*Args)
	if cfg.LocalConfig == "" {
		return nil, fmt.Errorf("%s: 'local_config' must be specified", PluginType)
	}

	transport := &http.Transport{
		Proxy: http.ProxyFromEnvironment,
		DialContext: (&net.Dialer{
			Timeout:   30 * time.Second,
			KeepAlive: 30 * time.Second,
		}).DialContext,
		TLSHandshakeTimeout: 10 * time.Second,
	}
	if cfg.Socks5 != "" {
		dialer, err := proxy.SOCKS5("tcp", cfg.Socks5, nil, proxy.Direct)
		if err != nil {
			return nil, fmt.Errorf("%s: failed to create SOCKS5 dialer: %w", PluginType, err)
		}
		contextDialer, ok := dialer.(proxy.ContextDialer)
		if !ok {
			return nil, fmt.Errorf("%s: created dialer does not support context", PluginType)
		}
		transport.DialContext = contextDialer.DialContext
		transport.Proxy = nil
	}
	httpClient := &http.Client{
		Timeout:   downloadTimeout,
		Transport: transport,
	}

	ctx, cancel := context.WithCancel(context.Background())
	p := &NftAdd{
		sources:         make(map[string]*RuleSource),
		localConfigFile: cfg.LocalConfig,
		nftArgs:         cfg.NftConfig,
		httpClient:      httpClient,
		ctx:             ctx,
		cancel:          cancel,
	}
	p.matcher.Store(netlist.NewList())

	if err := p.loadConfig(); err != nil {
		log.Printf("[%s] failed to load config file: %v", PluginType, err)
	}
	if err := p.reloadAllRules(); err != nil {
		log.Printf("[%s] failed to perform initial rule load: %v", PluginType, err)
	}
	bp.RegAPI(p.api())
	go p.backgroundUpdater()
	if p.nftArgs.Enable == "nft_true" {
		go p.startupNftRoutine()
	}
	return p, nil
}
func (p *NftAdd) Match(addr netip.Addr) bool {
	m, ok := p.matcher.Load().(netlist.Matcher)
	if !ok || m == nil {
		return false
	}
	return m.Match(addr)
}

func (p *NftAdd) GetIPMatcher() netlist.Matcher {
	return p
}

func (p *NftAdd) Close() error {
	log.Printf("[%s] closing...", PluginType)
	p.cancel()
	if p.ebpfLink != nil {
		p.ebpfLink.Close()
	}
	if p.ebpfMap != nil {
		p.ebpfMap.Close()
	}
	p.cleanupRouting()
	return nil
}

// -----------------------------------------------------------------------------
// NFT Core Logic
// -----------------------------------------------------------------------------

func (p *NftAdd) startupNftRoutine() {
	delay := time.Duration(p.nftArgs.StartupDelay) * time.Second
	if delay <= 0 {
		delay = 1 * time.Second
	}
	log.Printf("[%s] NFT startup sequence initiated. Waiting %v...", PluginType, delay)

	select {
	case <-time.After(delay):
	case <-p.ctx.Done():
		return
	}

	log.Printf("[%s] Phase 1: Preparing IP data (Streaming Mode)...", PluginType)
	ipSet, err := p.buildFullIPSet()
	if err != nil {
		log.Printf("[%s] FATAL: Startup aborted. Failed to build IP set: %v", PluginType, err)
		return
	}

	if p.nftArgs.Enable == "nft_true" {
		log.Printf("[%s] Phase 2: Resetting firewall table '%s'...", PluginType, p.nftArgs.Table)
		exec.Command("nft", "delete", "table", p.nftArgs.TableFamily, p.nftArgs.Table).Run()

		if _, err := os.Stat(p.nftArgs.NftConfFile); err == nil {
			loadCmd := exec.Command("nft", "-f", p.nftArgs.NftConfFile)
			if out, err := loadCmd.CombinedOutput(); err != nil {
				log.Printf("[%s] FATAL: Failed to load nft config %s: %v. Output: %s", PluginType, p.nftArgs.NftConfFile, err, string(out))
				return
			}
		}

		log.Printf("[%s] Phase 3: Injecting IP sets...", PluginType)
		if err := p.flushAndFillSets(ipSet); err != nil {
			log.Printf("[%s] ERROR: Failed to inject IPs: %v", PluginType, err)
		}
	}

	if p.nftArgs.EbpfEnable == "ebpf_true" {
		log.Printf("[%s] Phase 4: Setting up eBPF on interface %s...", PluginType, p.nftArgs.EbpfIface)
		if err := p.setupEbpf(ipSet); err != nil {
			log.Printf("[%s] ERROR: eBPF setup failed: %v", PluginType, err)
		} else {
			log.Printf("[%s] eBPF production proxy fully operational.", PluginType)
		}
		coremain.ManualGC() 
	}

	p.startupDone.Store(true)
	log.Printf("[%s] Startup sequence completed.", PluginType)
	coremain.ManualGC()
}

func (p *NftAdd) buildFullIPSet() (*netipx.IPSet, error) {
	var builder netipx.IPSetBuilder
	receiver := &builderReceiver{builder: &builder}

	p.mu.RLock()
	var enabledFiles []string
	for _, src := range p.sources {
		if src.Enabled && src.Files != "" {
			enabledFiles = append(enabledFiles, src.Files)
		}
	}
	p.mu.RUnlock()

	for _, fpath := range enabledFiles {
		f, err := os.Open(fpath)
		if err != nil {
			log.Printf("[%s] WARN: failed to open SRS %s: %v", PluginType, fpath, err)
			continue
		}
		ok, _, _ := tryLoadSRSStream(f, receiver)
		f.Close()
		if !ok {
			return nil, fmt.Errorf("invalid srs file: %s", fpath)
		}
	}

	if p.nftArgs.FixIPFile != "" {
		if err := loadFixIPToReceiver(p.nftArgs.FixIPFile, receiver); err != nil {
			return nil, fmt.Errorf("load fixip failed: %w", err)
		}
	}

	return builder.IPSet()
}

func (p *NftAdd) flushAndFillSets(ipSet *netipx.IPSet) error {
	var v4List, v6List []string
	for _, pfx := range ipSet.Prefixes() {
		if pfx.Addr().Is4() {
			v4List = append(v4List, pfx.String())
		} else {
			v6List = append(v6List, pfx.String())
		}
	}

	var script strings.Builder
	if p.nftArgs.SetV4 != "" {
		script.WriteString(fmt.Sprintf("flush set %s %s %s\n", p.nftArgs.TableFamily, p.nftArgs.Table, p.nftArgs.SetV4))
		if len(v4List) > 0 {
			script.WriteString(fmt.Sprintf("add element %s %s %s { %s }\n", p.nftArgs.TableFamily, p.nftArgs.Table, p.nftArgs.SetV4, strings.Join(v4List, ", ")))
		}
	}
	if p.nftArgs.SetV6 != "" {
		script.WriteString(fmt.Sprintf("flush set %s %s %s\n", p.nftArgs.TableFamily, p.nftArgs.Table, p.nftArgs.SetV6))
		if len(v6List) > 0 {
			script.WriteString(fmt.Sprintf("add element %s %s %s { %s }\n", p.nftArgs.TableFamily, p.nftArgs.Table, p.nftArgs.SetV6, strings.Join(v6List, ", ")))
		}
	}

	if script.Len() == 0 {
		return nil
	}

	cmd := exec.Command("nft", "-f", "-")
	cmd.Stdin = strings.NewReader(script.String())
	output, err := cmd.CombinedOutput()
	if err != nil {
		return fmt.Errorf("nft execution failed: %v, output: %s", err, string(output))
	}
	return nil
}

// -----------------------------------------------------------------------------
// eBPF Helper Logic
// -----------------------------------------------------------------------------

func (p *NftAdd) runCmd(cmd string) {
	_ = exec.Command("sh", "-c", cmd).Run()
}

func (p *NftAdd) setupRouting() {
	p.runCmd("ip rule del pref 1 2>/dev/null")
	p.runCmd("ip rule del pref 2 2>/dev/null")
	p.runCmd("ip rule del pref 3 2>/dev/null")
	p.runCmd("ip rule del pref 4 2>/dev/null")
	p.runCmd("ip rule del pref 5 2>/dev/null")
	p.runCmd("ip -6 rule del pref 1 2>/dev/null")
	p.runCmd("ip -6 rule del pref 2 2>/dev/null")
	p.runCmd("ip -6 rule del pref 3 2>/dev/null")
	p.runCmd("ip -6 rule del pref 4 2>/dev/null")
	p.runCmd("ip -6 rule del pref 5 2>/dev/null")

	p.runCmd("ip rule add pref 1 iif lo to 10.0.0.0/8 lookup main")
	p.runCmd("ip rule add pref 2 iif lo to 172.16.0.0/12 lookup main")
	p.runCmd("ip rule add pref 3 iif lo to 192.168.0.0/16 lookup main")
	p.runCmd("ip rule add pref 4 fwmark 1 table 100")
	p.runCmd("ip rule add pref 5 fwmark 2 table 101")
	p.runCmd("ip route add local default dev lo table 100 2>/dev/null")
	p.runCmd("ip route add local default dev lo table 101 2>/dev/null")

	p.runCmd("ip -6 rule add pref 2 iif lo to fe80::/10 lookup main")
	p.runCmd("ip -6 rule add pref 3 fwmark 1 table 200")
	p.runCmd("ip -6 rule add pref 4 fwmark 2 table 201")
	p.runCmd("ip -6 route add local default dev lo table 200 2>/dev/null")
	p.runCmd("ip -6 route add local default dev lo table 201 2>/dev/null")

	p.runCmd("sysctl -w net.ipv4.ip_forward=1")
	p.runCmd("sysctl -w net.ipv4.conf.all.rp_filter=0")
	p.runCmd("sysctl -w net.ipv4.conf.lo.rp_filter=0")
	p.runCmd("sysctl -w net.ipv4.conf.all.accept_local=1")
	p.runCmd("sysctl -w net.ipv4.conf.lo.accept_local=1")
	p.runCmd("sysctl -w net.ipv4.conf.all.route_localnet=1")
}

func (p *NftAdd) cleanupRouting() {
	p.runCmd("ip rule del pref 1 2>/dev/null")
	p.runCmd("ip rule del pref 2 2>/dev/null")
	p.runCmd("ip rule del pref 3 2>/dev/null")
	p.runCmd("ip rule del pref 4 2>/dev/null")
	p.runCmd("ip rule del pref 5 2>/dev/null")
	p.runCmd("ip -6 rule del pref 1 2>/dev/null")
	p.runCmd("ip -6 rule del pref 2 2>/dev/null")
	p.runCmd("ip -6 rule del pref 3 2>/dev/null")
	p.runCmd("ip -6 rule del pref 4 2>/dev/null")
	p.runCmd("ip -6 rule del pref 5 2>/dev/null")
}

func (p *NftAdd) packLpmKey(prefix netip.Prefix) []byte {
	key := make([]byte, 20)
	binary.LittleEndian.PutUint32(key[0:4], uint32(prefix.Bits()))
	addr := prefix.Addr().As16()
	copy(key[4:20], addr[:])
	if prefix.Addr().Is4() {
		v4 := prefix.Addr().As4()
		copy(key[4:8], v4[:])
		for i := 8; i < 20; i++ {
			key[i] = 0
		}
	}
	return key
}

func (p *NftAdd) setupEbpf(ipSet *netipx.IPSet) error {
	if p.ebpfLink != nil {
		p.ebpfLink.Close()
		p.ebpfLink = nil
	}
	if p.ebpfMap != nil {
		p.ebpfMap.Close()
		p.ebpfMap = nil
	}

	p.setupRouting()

	spec, err := ebpf.LoadCollectionSpecFromReader(bytes.NewReader(ebpfProg))
	if err != nil {
		return err
	}

	consts := make(map[string]interface{})
	if p.nftArgs.MihomoPort != 0 {
		consts["mihomo_port"] = p.nftArgs.MihomoPort
	}
	if p.nftArgs.SingboxPort != 0 {
		consts["singbox_port"] = p.nftArgs.SingboxPort
	}
	if len(consts) > 0 {
		spec.RewriteConstants(consts)
	}

	var objs struct {
		IngressL2  *ebpf.Program `ebpf:"tc_ingress_l2"`
		RouteRules *ebpf.Map     `ebpf:"route_rules"`
	}

	if err := spec.LoadAndAssign(&objs, nil); err != nil {
		return err
	}
	spec = nil 
	p.ebpfMap = objs.RouteRules

	lan, err := net.InterfaceByName(p.nftArgs.EbpfIface)
	if err != nil {
		objs.IngressL2.Close()
		objs.RouteRules.Close()
		return err
	}

	l, err := link.AttachTCX(link.TCXOptions{
		Interface: lan.Index,
		Program:   objs.IngressL2,
		Attach:    ebpf.AttachTCXIngress,
	})
	if err != nil {
		objs.IngressL2.Close()
		objs.RouteRules.Close()
		return err
	}
	p.ebpfLink = l
	objs.IngressL2 = nil 
	return p.syncEbpfMap(ipSet)
}

func (p *NftAdd) syncEbpfMap(ipSet *netipx.IPSet) error {
	if p.ebpfMap == nil {
		return nil
	}

	mark2 := uint32(2)
	fakeIPs := []string{p.nftArgs.MihomoFakeIPv4, p.nftArgs.MihomoFakeIPv6}
	for _, cidr := range fakeIPs {
		if cidr == "" {
			continue
		}
		if prefix, err := netip.ParsePrefix(cidr); err == nil {
			p.ebpfMap.Update(p.packLpmKey(prefix), &mark2, ebpf.UpdateAny)
		}
	}

	mark1 := uint32(1)
	for _, pfx := range ipSet.Prefixes() {
		p.ebpfMap.Update(p.packLpmKey(pfx), &mark1, ebpf.UpdateAny)
	}
	return nil
}
// -----------------------------------------------------------------------------
// Helper functions: file parsing (Streaming Mode)
// -----------------------------------------------------------------------------

func loadFixIPToReceiver(path string, receiver IPReceiver) error {
	f, err := os.Open(path)
	if err != nil {
		if os.IsNotExist(err) {
			log.Printf("[%s] WARN: fixip file %s not found, skipping.", PluginType, path)
			return nil
		}
		return err
	}
	defer f.Close()

	scanner := bufio.NewScanner(f)
	lineNum := 0
	for scanner.Scan() {
		lineNum++
		line := strings.TrimSpace(scanner.Text())
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		
		if pfx, err := netip.ParsePrefix(line); err == nil {
			receiver.AddPrefix(pfx)
			continue
		}
		if addr, err := netip.ParseAddr(line); err == nil {
			receiver.AddPrefix(netip.PrefixFrom(addr, addr.BitLen()))
			continue
		}
		log.Printf("[%s] WARN: invalid ip/cidr in fixip file line %d: %s", PluginType, lineNum, line)
	}
	return scanner.Err()
}

func tryLoadSRSStream(r io.Reader, receiver IPReceiver) (bool, int, string) {
	var mb [3]byte
	if _, err := io.ReadFull(r, mb[:]); err != nil || mb != srsMagic {
		return false, 0, ""
	}
	var version uint8
	if err := binary.Read(r, binary.BigEndian, &version); err != nil || version > maxSupportedVersion {
		return false, 0, ""
	}
	zr, err := zlib.NewReader(r)
	if err != nil {
		return false, 0, ""
	}
	defer zr.Close()
	br := bufio.NewReader(zr)

	length, err := binary.ReadUvarint(br)
	if err != nil {
		return false, 0, ""
	}
	count := 0
	var last string
	for i := uint64(0); i < length; i++ {
		c, lr := readRuleToReceiver(br, receiver)
		if lr != "" {
			last = lr
		}
		count += c
	}
	return true, count, last
}

func readRuleToReceiver(r *bufio.Reader, receiver IPReceiver) (int, string) {
	ct := 0
	var last string
	mode, err := r.ReadByte()
	if err != nil {
		return 0, ""
	}
	switch mode {
	case 0:
		c, lr := readDefaultToReceiver(r, receiver)
		ct += c
		if lr != "" { last = lr }
	case 1:
		_, _ = r.ReadByte()
		n, _ := binary.ReadUvarint(r)
		for j := uint64(0); j < n; j++ {
			c, lr := readRuleToReceiver(r, receiver)
			ct += c
			if lr != "" { last = lr }
		}
		_, _ = r.ReadByte()
	}
	return ct, last
}

func readDefaultToReceiver(r *bufio.Reader, receiver IPReceiver) (int, string) {
	count := 0
	var last string
	for {
		item, err := r.ReadByte()
		if err != nil || item == ruleItemFinal {
			break
		}
		switch item {
		case ruleItemIPCIDR:
			ipset, err := parseIPSet(r)
			if err != nil {
				return count, last
			}
			for _, pfx := range ipset.Prefixes() {
				receiver.AddPrefix(pfx)
				count++
				last = pfx.String()
			}
		default:
			// unknown item
		}
	}
	return count, last
}

func parseIPSet(r varbin.Reader) (netipx.IPSet, error) {
	ver, err := r.ReadByte()
	if err != nil || ver != 1 {
		return netipx.IPSet{}, fmt.Errorf("unsupported ipset version")
	}
	var length uint64
	if err := binary.Read(r, binary.BigEndian, &length); err != nil {
		return netipx.IPSet{}, err
	}
	type ipRangeData struct{ From, To []byte }
	ranges := make([]ipRangeData, length)
	if err := varbin.Read(r, binary.BigEndian, &ranges); err != nil {
		return netipx.IPSet{}, err
	}

	var builder netipx.IPSetBuilder
	for _, rr := range ranges {
		from, ok := netip.AddrFromSlice(rr.From)
		if !ok { continue }
		to, ok := netip.AddrFromSlice(rr.To)
		if !ok { continue }
		builder.AddRange(netipx.IPRangeFrom(from, to))
	}
	pPtr, err := builder.IPSet()
	if err != nil {
		return netipx.IPSet{}, err
	}
	return *pPtr, nil
}

// -----------------------------------------------------------------------------
// Logic: Config, Updater, API
// -----------------------------------------------------------------------------

func (p *NftAdd) loadConfig() error {
	p.mu.Lock()
	defer p.mu.Unlock()
	data, err := os.ReadFile(p.localConfigFile)
	if err != nil {
		if os.IsNotExist(err) { return nil }
		return err
	}
	if len(data) == 0 { return nil }
	var sources []*RuleSource
	if err := json.Unmarshal(data, &sources); err != nil {
		return fmt.Errorf("failed to parse config json: %w", err)
	}
	p.sources = make(map[string]*RuleSource, len(sources))
	for _, src := range sources {
		if src.Name != "" { p.sources[src.Name] = src }
	}
	return nil
}

func (p *NftAdd) saveConfig() error {
	p.mu.Lock()
	defer p.mu.Unlock()
	sourcesSnapshot := make([]*RuleSource, 0, len(p.sources))
	for _, src := range p.sources {
		s := *src
		sourcesSnapshot = append(sourcesSnapshot, &s)
	}
	sort.Slice(sourcesSnapshot, func(i, j int) bool {
		return sourcesSnapshot[i].Name < sourcesSnapshot[j].Name
	})
	data, err := json.MarshalIndent(sourcesSnapshot, "", "  ")
	if err != nil { return err }
	tmpFile := p.localConfigFile + ".tmp"
	if err := os.WriteFile(tmpFile, data, 0644); err != nil { return err }
	return os.Rename(tmpFile, p.localConfigFile)
}

func (p *NftAdd) reloadAllRules() error {
	p.mu.RLock()
	enabledSources := make([]*RuleSource, 0, len(p.sources))
	for _, src := range p.sources {
		if src.Enabled { enabledSources = append(enabledSources, src) }
	}
	p.mu.RUnlock()

	newList := netlist.NewList()
	totalRules := 0
	configChanged := false
	lReceiver := &listReceiver{l: newList}

	for _, src := range enabledSources {
		if src.Files == "" { continue }
		f, err := os.Open(src.Files)
		if err != nil {
			log.Printf("[%s] ERROR opening %s: %v", PluginType, src.Files, err)
			continue
		}
		ok, count, _ := tryLoadSRSStream(f, lReceiver)
		f.Close()
		if ok {
			totalRules += count
			if src.RuleCount != count {
				p.mu.Lock()
				if s, ok := p.sources[src.Name]; ok {
					s.RuleCount = count
					configChanged = true
				}
				p.mu.Unlock()
			}
		}
	}

	newList.Sort()
	p.matcher.Store(newList)
	log.Printf("[%s] DNS rules reloaded. Total: %d", PluginType, totalRules)
	if configChanged { p.saveConfig() }

	if p.startupDone.Load() && p.nftArgs.Enable == "nft_true" {
		go func() {
			log.Printf("[%s] Triggering NFT/eBPF sync after reload...", PluginType)
			ipSet, err := p.buildFullIPSet()
			if err != nil { return }
			p.flushAndFillSets(ipSet)
			if p.nftArgs.EbpfEnable == "ebpf_true" {
				if err := p.setupEbpf(ipSet); err != nil {
					log.Printf("[%s] ERROR: eBPF resync failed: %v", PluginType, err)
				}
			}
			coremain.ManualGC()
		}()
	}
	return nil
}

func (p *NftAdd) downloadAndUpdateLocalFile(ctx context.Context, sourceName string) error {
	p.mu.RLock()
	source, ok := p.sources[sourceName]
	if !ok { p.mu.RUnlock(); return fmt.Errorf("source not found") }
	url := source.URL
	localFile := source.Files
	p.mu.RUnlock()
	req, err := http.NewRequestWithContext(ctx, "GET", url, nil)
	if err != nil { return err }
	resp, err := p.httpClient.Do(req)
	if err != nil { return err }
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK { return fmt.Errorf("http %d", resp.StatusCode) }
	data, err := io.ReadAll(resp.Body)
	if err != nil { return err }
	counter := &counterReceiver{}
	ok, count, _ := tryLoadSRSStream(bytes.NewReader(data), counter)
	if !ok { return fmt.Errorf("invalid srs data") }
	os.MkdirAll(filepath.Dir(localFile), 0755)
	if err := os.WriteFile(localFile, data, 0644); err != nil { return err }
	p.mu.Lock()
	if s, ok := p.sources[sourceName]; ok {
		s.RuleCount = count
		s.LastUpdated = time.Now()
	}
	p.mu.Unlock()
	return p.saveConfig()
}

func (p *NftAdd) backgroundUpdater() {
	select {
	case <-time.After(1 * time.Minute):
	case <-p.ctx.Done(): return
	}
	ticker := time.NewTicker(10 * time.Minute)
	defer ticker.Stop()
	for {
		select {
		case <-ticker.C:
			p.mu.RLock()
			var toUpdate []string
			for name, src := range p.sources {
				if src.Enabled && src.AutoUpdate && src.UpdateIntervalHours > 0 {
					if time.Since(src.LastUpdated).Hours() >= float64(src.UpdateIntervalHours) {
						toUpdate = append(toUpdate, name)
					}
				}
			}
			p.mu.RUnlock()
			if len(toUpdate) > 0 {
				var wg sync.WaitGroup
				for _, name := range toUpdate {
					wg.Add(1)
					go func(n string) {
						defer wg.Done()
						ctx, cancel := context.WithTimeout(p.ctx, downloadTimeout)
						defer cancel()
						if err := p.downloadAndUpdateLocalFile(ctx, n); err != nil {
							log.Printf("[%s] auto-update failed for %s: %v", PluginType, n, err)
						}
					}(name)
				}
				wg.Wait()
				p.reloadAllRules()
				coremain.ManualGC()
			}
		case <-p.ctx.Done(): return
		}
	}
}

func (p *NftAdd) api() *chi.Mux {
	r := chi.NewRouter()
	r.Get("/config", func(w http.ResponseWriter, r *http.Request) {
		p.mu.RLock()
		defer p.mu.RUnlock()
		var sources []*RuleSource
		for _, s := range p.sources { sources = append(sources, s) }
		sort.Slice(sources, func(i, j int) bool { return sources[i].Name < sources[j].Name })
		jsonResponse(w, sources, 200)
	})
	r.Post("/update/{name}", func(w http.ResponseWriter, r *http.Request) {
		name := chi.URLParam(r, "name")
		go func() {
			ctx, cancel := context.WithTimeout(p.ctx, downloadTimeout*2)
			defer cancel()
			if err := p.downloadAndUpdateLocalFile(ctx, name); err == nil { p.reloadAllRules() }
		}()
		jsonResponse(w, map[string]string{"status": "started"}, 202)
	})
	r.Put("/config/{name}", func(w http.ResponseWriter, r *http.Request) {
		name := chi.URLParam(r, "name")
		var req RuleSource
		if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
			jsonError(w, "bad request", 400); return
		}
		req.Name = name
		p.mu.Lock()
		if existing, ok := p.sources[name]; ok {
			existing.Type = req.Type
			existing.Files = req.Files
			existing.URL = req.URL
			existing.Enabled = req.Enabled
			existing.AutoUpdate = req.AutoUpdate
			existing.UpdateIntervalHours = req.UpdateIntervalHours
		} else {
			p.sources[name] = &req
		}
		p.mu.Unlock()
		p.saveConfig()
		go p.reloadAllRules()
		jsonResponse(w, req, 200)
	})
	r.Delete("/config/{name}", func(w http.ResponseWriter, r *http.Request) {
		name := chi.URLParam(r, "name")
		p.mu.Lock()
		src, ok := p.sources[name]
		if ok {
			delete(p.sources, name)
			os.Remove(src.Files)
		}
		p.mu.Unlock()
		if ok {
			p.saveConfig()
			go p.reloadAllRules()
			w.WriteHeader(204)
		} else { w.WriteHeader(404) }
	})
	return r
}

func jsonResponse(w http.ResponseWriter, v any, code int) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(code)
	json.NewEncoder(w).Encode(v)
}

func jsonError(w http.ResponseWriter, msg string, code int) {
	jsonResponse(w, map[string]string{"error": msg}, code)
}

var (
	srsMagic            = [3]byte{'S', 'R', 'S'}
	ruleItemIPCIDR      = uint8(6)
	ruleItemFinal       = uint8(0xFF)
	maxSupportedVersion = uint8(3)
)