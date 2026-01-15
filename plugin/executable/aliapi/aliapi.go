/*
 * Copyright (C) 2020-2022, IrineSistiana
 *
 * This file is part of mosdns.
 */

package aliapi

import (
	"context"
	"crypto/sha256"
	"crypto/tls"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"math/rand"
	"net"
	"net/http"
	"strconv"
	"strings"
	"time"

	"github.com/IrineSistiana/mosdns/v5/coremain"
	"github.com/IrineSistiana/mosdns/v5/pkg/pool"
	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
	"github.com/IrineSistiana/mosdns/v5/pkg/upstream"
	"github.com/IrineSistiana/mosdns/v5/pkg/utils"
	"github.com/IrineSistiana/mosdns/v5/plugin/executable/sequence"
	"github.com/miekg/dns"
	"github.com/prometheus/client_golang/prometheus"
	"go.uber.org/zap"
)

const PluginType = "aliapi"

func init() {
	coremain.RegNewPluginFunc(PluginType, Init, func() any { return new(Args) })
	sequence.MustRegExecQuickSetup(PluginType, quickSetup)
}

const (
	maxConcurrentQueries = 20
	queryTimeout         = time.Second * 5
	defaultAliAPIServer  = "223.5.5.5"
)

// Args defines the configuration for the aliapi plugin.
type Args struct {
	Upstreams []UpstreamConfig `yaml:"upstreams"`
	Concurrent int              `yaml:"concurrent"`

	// AliDNS API specific options, global to this plugin instance.
	AccountID       string `yaml:"account_id"`
	AccessKeyID     string `yaml:"access_key_id"`
	AccessKeySecret string `yaml:"access_key_secret"`
	ServerAddr      string `yaml:"server_addr"`
	EcsClientIP     string `yaml:"ecs_client_ip"`
	EcsClientMask   uint8  `yaml:"ecs_client_mask"`

	// Global options for standard DNS upstreams.
	Socks5       string `yaml:"socks5"`
	SoMark       int    `yaml:"so_mark"`
	BindToDevice string `yaml:"bind_to_device"`
	Bootstrap    string `yaml:"bootstrap"`
	BootstrapVer int    `yaml:"bootstrap_version"`
}

// UpstreamConfig defines a single upstream server configuration.
type UpstreamConfig struct {
	Tag                  string `yaml:"tag"`
	Addr                 string `yaml:"addr"`
	DialAddr             string `yaml:"dial_addr"`
	IdleTimeout          int    `yaml:"idle_timeout"`
	UpstreamQueryTimeout int    `yaml:"upstream_query_timeout"`

	Type string `yaml:"type"` // "dns" (default) or "aliapi"

	// Deprecated: This option has no affect.
	// TODO: (v6) Remove this option.
	MaxConns           int  `yaml:"max_conns"`
	EnablePipeline     bool `yaml:"enable_pipeline"`
	EnableHTTP3        bool `yaml:"enable_http3"`
	InsecureSkipVerify bool `yaml:"insecure_skip_verify"`

	Socks5       string `yaml:"socks5"`
	SoMark       int    `yaml:"so_mark"`
	BindToDevice string `yaml:"bind_to_device"`
	Bootstrap    string `yaml:"bootstrap"`
	BootstrapVer int    `yaml:"bootstrap_version"`
}

func Init(bp *coremain.BP, args any) (any, error) {
	a := args.(*Args)
	
	// [Debug] Log entry
	bp.L().Info("[Debug AliAPI] Init plugin instance", zap.String("plugin_tag", bp.Tag()))

	// 从 api_upstream.go 获取覆盖配置
	overrides := coremain.GetUpstreamOverrides(bp.Tag())
	
	var activeUpstreams []UpstreamConfig
	enabledCount := 0

	if overrides != nil && len(overrides) > 0 {
		bp.L().Info("[Debug AliAPI] Found upstream overrides", zap.String("tag", bp.Tag()), zap.Int("count", len(overrides))) // DEBUG

		for _, o := range overrides {
			if !o.Enabled {
				continue
			}
			
			u := UpstreamConfig{
				Tag: o.Tag, Addr: o.Addr, DialAddr: o.DialAddr,
				IdleTimeout: o.IdleTimeout, UpstreamQueryTimeout: o.UpstreamQueryTimeout,
				EnablePipeline: o.EnablePipeline, EnableHTTP3: o.EnableHTTP3,
				InsecureSkipVerify: o.InsecureSkipVerify, Socks5: o.Socks5,
				SoMark: o.SoMark, BindToDevice: o.BindToDevice,
				Bootstrap: o.Bootstrap, BootstrapVer: o.BootstrapVer,
			}

			if o.Protocol == "aliapi" {
				u.Type = "aliapi"
				// 如果配置了 aliapi 类型的上游，且提供了凭证，覆盖全局 Args
				if o.AccountID != "" {
					a.AccountID = o.AccountID
					a.AccessKeyID = o.AccessKeyID
					a.AccessKeySecret = o.AccessKeySecret
					a.ServerAddr = o.ServerAddr
					a.EcsClientIP = o.EcsClientIP
					a.EcsClientMask = o.EcsClientMask
				}
			} else {
				u.Type = "dns"
				// Addr 已经包含了协议头 (前端处理) 或者在 NewUpstream 自动处理
			}
			activeUpstreams = append(activeUpstreams, u)
			enabledCount++
		}

		// 需求：只有当覆盖配置中有至少一个启用条目时，才替换原始配置
		if len(activeUpstreams) > 0 {
			a.Upstreams = activeUpstreams
			
			// 需求：自动设置 concurrent 数量（1-3）
			conc := enabledCount
			if conc > maxConcurrentQueries { conc = maxConcurrentQueries }
			if conc < 1 { conc = 1 }
			a.Concurrent = conc
			
			bp.L().Info("[Debug AliAPI] Configuration REPLACED by overrides", 
				zap.String("tag", bp.Tag()),
				zap.Int("active_upstreams", enabledCount),
				zap.Int("new_concurrent", a.Concurrent))
		} else {
			bp.L().Info("[Debug AliAPI] Overrides exist but none enabled, using default YAML config", zap.String("tag", bp.Tag()))
		}
	} else {
		bp.L().Info("[Debug AliAPI] No overrides found, using default YAML config", zap.String("tag", bp.Tag()))
	}

	// 执行正常初始化
	f, err := NewAliAPI(a, Opts{Logger: bp.L(), MetricsTag: bp.Tag()})
	if err != nil {
		return nil, err
	}
	if err := f.RegisterMetricsTo(prometheus.WrapRegistererWithPrefix(PluginType+"_", bp.M().GetMetricsReg())); err != nil {
		_ = f.Close()
		return nil, err
	}
	return f, nil
}

// applyUpstreamOverrides 负责根据本地 JSON 文件内容动态修改插件运行参数
// (保留此函数以防你在其他地方调用，但逻辑已移至 Init)
func applyUpstreamOverrides(pluginTag string, args *Args, logger *zap.Logger) {
	// ... 略 (因为 Init 已经处理了)
}

var _ sequence.Executable = (*AliAPI)(nil)
var _ sequence.QuickConfigurableExec = (*AliAPI)(nil)

// AliAPI represents the aliapi plugin instance.
type AliAPI struct {
	args *Args

	logger       *zap.Logger
	us           []*upstreamWrapper
	tag2Upstream map[string]*upstreamWrapper
}

type Opts struct {
	Logger     *zap.Logger
	MetricsTag string
}

// NewAliAPI inits a AliAPI from given args.
// args must contain at least one upstream.
func NewAliAPI(args *Args, opt Opts) (*AliAPI, error) {
	if len(args.Upstreams) == 0 {
		return nil, errors.New("no upstream is configured")
	}
	if opt.Logger == nil {
		opt.Logger = zap.NewNop()
	}

	if args.ServerAddr == "" {
		args.ServerAddr = defaultAliAPIServer
	}

	f := &AliAPI{
		args:         args,
		logger:       opt.Logger,
		tag2Upstream: make(map[string]*upstreamWrapper),
	}

	applyGlobal := func(c *UpstreamConfig) {
		utils.SetDefaultString(&c.Socks5, args.Socks5)
		utils.SetDefaultUnsignNum(&c.SoMark, args.SoMark)
		utils.SetDefaultString(&c.BindToDevice, args.BindToDevice)
		utils.SetDefaultString(&c.Bootstrap, args.Bootstrap)
		utils.SetDefaultUnsignNum(&c.BootstrapVer, args.BootstrapVer)
		utils.SetDefaultString(&c.Type, "dns")
	}

	for i, c := range args.Upstreams {
		applyGlobal(&c)

		uw := newWrapper(i, c, opt.MetricsTag)
		var u upstream.Upstream
		var err error

		if c.Type == "aliapi" {
			if args.AccountID == "" || args.AccessKeyID == "" || args.AccessKeySecret == "" {
				return nil, fmt.Errorf("aliapi upstream requires account_id, access_key_id, and access_key_secret to be set in plugin args")
			}
			aliAPIArgs := AliAPIUpstreamArgs{
				AccountID:       args.AccountID,
				AccessKeyID:     args.AccessKeyID,
				AccessKeySecret: args.AccessKeySecret,
				ServerAddr:      args.ServerAddr,
				EcsClientIP:     args.EcsClientIP,
				EcsClientMask:   args.EcsClientMask,
			}
			u = NewAliAPIUpstream(aliAPIArgs, opt.Logger)
		} else {
			if len(c.Addr) == 0 {
				return nil, fmt.Errorf("#%d upstream invalid args, addr is required for type 'dns'", i)
			}
			uOpt := upstream.Opt{
				DialAddr:       c.DialAddr,
				Socks5:         c.Socks5,
				SoMark:         c.SoMark,
				BindToDevice:   c.BindToDevice,
				IdleTimeout:    time.Duration(c.IdleTimeout) * time.Second,
				EnablePipeline: c.EnablePipeline,
				EnableHTTP3:    c.EnableHTTP3,
				Bootstrap:      c.Bootstrap,
				BootstrapVer:   c.BootstrapVer,
				TLSConfig: &tls.Config{
					InsecureSkipVerify: c.InsecureSkipVerify,
					ClientSessionCache: tls.NewLRUClientSessionCache(4),
				},
				Logger:        opt.Logger,
				EventObserver: uw,
			}
			u, err = upstream.NewUpstream(c.Addr, uOpt)
			if err != nil {
				_ = f.Close()
				return nil, fmt.Errorf("failed to init upstream #%d: %w", i, err)
			}
		}

		uw.u = u
		f.us = append(f.us, uw)

		if len(c.Tag) > 0 {
			if _, dup := f.tag2Upstream[c.Tag]; dup {
				_ = f.Close()
				return nil, fmt.Errorf("duplicated upstream tag %s", c.Tag)
			}
			f.tag2Upstream[c.Tag] = uw
		}
	}

	return f, nil
}

func (f *AliAPI) RegisterMetricsTo(r prometheus.Registerer) error {
	for _, wu := range f.us {
		if len(wu.cfg.Tag) == 0 {
			continue
		}
		if err := wu.registerMetricsTo(r); err != nil {
			return err
		}
	}
	return nil
}

func (f *AliAPI) Exec(ctx context.Context, qCtx *query_context.Context) (err error) {
	r, err := f.exchange(ctx, qCtx, f.us)
	if err != nil {
		return err
	}
	qCtx.SetResponse(r)
	return nil
}

func (f *AliAPI) QuickConfigureExec(args string) (any, error) {
	var us []*upstreamWrapper
	if len(args) == 0 {
		us = f.us
	} else {
		for _, tag := range strings.Fields(args) {
			u := f.tag2Upstream[tag]
			if u == nil {
				return nil, fmt.Errorf("cannot find upstream by tag %s", tag)
			}
			us = append(us, u)
		}
	}
	var execFunc sequence.ExecutableFunc = func(ctx context.Context, qCtx *query_context.Context) error {
		r, err := f.exchange(ctx, qCtx, us)
		if err != nil {
			return err
		}
		qCtx.SetResponse(r)
		return nil
	}
	return execFunc, nil
}

func (f *AliAPI) Close() error {
	for _, u := range f.us {
		_ = u.Close()
	}
	return nil
}

func (f *AliAPI) exchange(ctx context.Context, qCtx *query_context.Context, us []*upstreamWrapper) (*dns.Msg, error) {
	if len(us) == 0 {
		return nil, errors.New("no upstream to exchange")
	}

	queryPayload, err := pool.PackBuffer(qCtx.Q())
	if err != nil {
		return nil, err
	}
	defer pool.ReleaseBuf(queryPayload)

	concurrent := f.args.Concurrent
	if concurrent <= 0 {
		concurrent = 1
	}
	if concurrent > maxConcurrentQueries {
		concurrent = maxConcurrentQueries
	}
	if concurrent > len(us) {
		concurrent = len(us)
	}

	type res struct {
		r   *dns.Msg
		err error
		u   *upstreamWrapper
	}

	resChan := make(chan res, concurrent)

	var lastSuccessOrNXRes *dns.Msg
	var lastSuccessOrNXResUpstream *upstreamWrapper
	var lastOtherRes *dns.Msg
	var lastOtherResUpstream *upstreamWrapper
	var lastError error

	randIndex := rand.Intn(len(us))

	usToQuery := make([]*upstreamWrapper, 0, concurrent)
	for i := 0; i < concurrent; i++ {
		usToQuery = append(usToQuery, us[(randIndex+i)%len(us)])
	}

	for _, u := range usToQuery {
		qc := copyPayload(queryPayload)

		upstreamTimeout := time.Duration(u.cfg.UpstreamQueryTimeout) * time.Millisecond
		if upstreamTimeout == 0 {
			upstreamTimeout = queryTimeout
		}

		go func(uqid uint32, question dns.Question, currentUpstream *upstreamWrapper) {
			defer pool.ReleaseBuf(qc)

			upstreamCtx, upstreamCancel := context.WithTimeout(context.Background(), upstreamTimeout)
			defer upstreamCancel()

			var r *dns.Msg
			respPayload, err := currentUpstream.ExchangeContext(upstreamCtx, *qc)
			
			if err != nil {
				if !errors.Is(err, context.DeadlineExceeded) && !errors.Is(err, context.Canceled) &&
					!strings.Contains(err.Error(), "connection refused") &&
					!strings.Contains(err.Error(), "no such host") {
					f.logger.Debug("upstream query failed", zap.String("upstream", currentUpstream.cfg.Addr), zap.Error(err))
				}
			} else {
				r = new(dns.Msg)
				err = r.Unpack(*respPayload)
				pool.ReleaseBuf(respPayload)
				if err != nil {
					r = nil
					f.logger.Debug("failed to unpack DNS response", zap.String("upstream", currentUpstream.cfg.Addr), zap.Error(err))
				}
			}

			select {
			case resChan <- res{r: r, err: err, u: currentUpstream}:
			case <-upstreamCtx.Done(): 
			}
		}(qCtx.Id(), qCtx.QQuestion(), u)
	}

	for i := 0; i < len(usToQuery); i++ {
		select {
		case res := <-resChan:
			r, err, u := res.r, res.err, res.u
			if err != nil {
				if lastError == nil {
					lastError = err
				}
				continue
			}

			if len(r.Answer) > 0 {
				for _, ans := range r.Answer {
					if a, ok := ans.(*dns.A); ok && len(a.A) > 0 {
						u.mWinnerTotal.Inc()
						return r, nil
					}
					if aaaa, ok := ans.(*dns.AAAA); ok && len(aaaa.AAAA) > 0 {
						u.mWinnerTotal.Inc()
						return r, nil
					}
				}
			}

			if r.Rcode == dns.RcodeSuccess || r.Rcode == dns.RcodeNameError {
				if lastSuccessOrNXRes == nil {
					lastSuccessOrNXRes = r
					lastSuccessOrNXResUpstream = u
				}
			} else {
				if lastOtherRes == nil {
					lastOtherRes = r
					lastOtherResUpstream = u
				}
			}

		case <-ctx.Done():
			if lastSuccessOrNXRes != nil {
				lastSuccessOrNXResUpstream.mWinnerTotal.Inc()
				return lastSuccessOrNXRes, nil
			}
			if lastOtherRes != nil {
				lastOtherResUpstream.mWinnerTotal.Inc()
				return lastOtherRes, nil
			}
			if lastError != nil {
				return nil, lastError
			}
			return nil, context.Cause(ctx)
		}
	}

	if lastSuccessOrNXRes != nil {
		lastSuccessOrNXResUpstream.mWinnerTotal.Inc()
		return lastSuccessOrNXRes, nil
	}
	if lastOtherRes != nil {
		lastOtherResUpstream.mWinnerTotal.Inc()
		return lastOtherRes, nil
	}
	if lastError != nil {
		return nil, lastError
	}

	return nil, errors.New("all upstreams failed or returned no usable response")
}

func quickSetup(bq sequence.BQ, s string) (any, error) {
	args := new(Args)
	args.Concurrent = maxConcurrentQueries
	for _, u := range strings.Fields(s) {
		args.Upstreams = append(args.Upstreams, UpstreamConfig{Addr: u, Type: "dns"})
	}
	return NewAliAPI(args, Opts{Logger: bq.L()})
}

// DNSEntity represents the structure of the JSON response from AliDNS API
type DNSEntity struct {
	Status int      `json:"status"`
	TC     bool     `json:"TC"`       // Truncated
	RD     bool     `json:"RD"`       // Recursion Desired
	RA     bool     `json:"RA"`       // Recursion Available
	AD     bool     `json:"AD"`       // Authenticated Data
	CD     bool     `json:"CD"`       // Checking Disabled
	Answer []Answer `json:"answer"`
	Remark string   `json:"remark"`
}

// Answer represents a single DNS record in the AliDNS API response
type Answer struct {
	Name string `json:"name"`
	Type uint16 `json:"type"`
	TTL  uint32 `json:"TTL"`
	Data string `json:"data"`
}

// getDNSRecord converts a JSON Answer object to a dns.RR
func getDNSRecord(ans Answer) dns.RR {
	header := dns.RR_Header{Name: ans.Name, Rrtype: ans.Type, Class: dns.ClassINET, Ttl: ans.TTL}
	switch ans.Type {
	case dns.TypeA:
		rr := new(dns.A)
		rr.Hdr = header
		rr.A = net.ParseIP(ans.Data)
		return rr
	case dns.TypeNS:
		rr := new(dns.NS)
		rr.Hdr = header
		rr.Ns = ans.Data
		return rr
	case dns.TypeCNAME:
		rr := new(dns.CNAME)
		rr.Hdr = header
		rr.Target = ans.Data
		return rr
	case dns.TypeSOA:
		rr := new(dns.SOA)
		rr.Hdr = header
		data := strings.Fields(ans.Data)
		if len(data) == 7 {
			rr.Ns = data[0]
			rr.Mbox = data[1]
			serial, err1 := strconv.ParseUint(data[2], 10, 32)
			refresh, err2 := strconv.ParseUint(data[3], 10, 32)
			retry, err3 := strconv.ParseUint(data[4], 10, 32)
			expire, err4 := strconv.ParseUint(data[5], 10, 32)
			minttl, err5 := strconv.ParseUint(data[6], 10, 32)
			if err1 != nil || err2 != nil || err3 != nil || err4 != nil || err5 != nil {
				return &dns.TXT{Hdr: header, Txt: []string{"Malformed SOA: invalid number format"}}
			}
			rr.Serial = uint32(serial)
			rr.Refresh = uint32(refresh)
			rr.Retry = uint32(retry)
			rr.Expire = uint32(expire)
			rr.Minttl = uint32(minttl)
		} else {
			return &dns.TXT{Hdr: header, Txt: []string{"Malformed SOA: wrong number of fields"}}
		}
		return rr
	case dns.TypeMX:
		rr := new(dns.MX)
		rr.Hdr = header
		data := strings.Fields(ans.Data)
		if len(data) == 2 {
			pref, err := strconv.ParseUint(data[0], 10, 16)
			if err != nil {
				return &dns.TXT{Hdr: header, Txt: []string{"Malformed MX: invalid preference"}}
			}
			rr.Preference = uint16(pref)
			rr.Mx = data[1]
		} else {
			return &dns.TXT{Hdr: header, Txt: []string{"Malformed MX: wrong number of fields"}}
		}
		return rr
	case dns.TypeTXT:
		rr := new(dns.TXT)
		rr.Hdr = header
		cleanedData := strings.Trim(ans.Data, "\"")
		rr.Txt = []string{cleanedData}
		return rr
	case dns.TypeAAAA:
		rr := new(dns.AAAA)
		rr.Hdr = header
		rr.AAAA = net.ParseIP(ans.Data)
		return rr
	case dns.TypeCAA:
		rr := new(dns.CAA)
		rr.Hdr = header
		data := strings.Fields(ans.Data)
		if len(data) == 3 {
			flag, err := strconv.ParseUint(data[0], 10, 8)
			if err != nil {
				return &dns.TXT{Hdr: header, Txt: []string{"Malformed CAA: invalid flag"}}
			}
			rr.Flag = uint8(flag)
			rr.Tag = data[1]
			rr.Value = strings.Trim(data[2], "\"")
		} else {
			return &dns.TXT{Hdr: header, Txt: []string{"Malformed CAA: wrong number of fields"}}
		}
		return rr
	default:
		rr := new(dns.TXT)
		rr.Hdr = header
		cleanedData := strings.Trim(ans.Data, "\"")
		rr.Txt = []string{fmt.Sprintf("Type %d: %s", ans.Type, cleanedData)}
		return rr
	}
}

// AliAPIUpstreamArgs holds configuration for an AliAPI upstream instance.
type AliAPIUpstreamArgs struct {
	AccountID       string
	AccessKeyID     string
	AccessKeySecret string
	ServerAddr      string
	EcsClientIP     string
	EcsClientMask   uint8
}

// AliAPIUpstream implements the upstream.Upstream interface for AliDNS JSON API.
type AliAPIUpstream struct {
	args   AliAPIUpstreamArgs
	logger *zap.Logger
	client *http.Client
}

// NewAliAPIUpstream creates a new AliAPIUpstream.
func NewAliAPIUpstream(args AliAPIUpstreamArgs, logger *zap.Logger) *AliAPIUpstream {
	httpClient := &http.Client{
		Timeout: queryTimeout,
	}
	return &AliAPIUpstream{
		args:   args,
		logger: logger,
		client: httpClient,
	}
}

// ExchangeContext performs a DNS query via AliDNS JSON API.
func (a *AliAPIUpstream) ExchangeContext(ctx context.Context, req []byte) (resp *[]byte, err error) {
	dnsMsg := new(dns.Msg)
	if err := dnsMsg.Unpack(req); err != nil {
		a.logger.Warn("failed to unpack DNS message for AliAPI", zap.Error(err))
		return nil, fmt.Errorf("failed to unpack DNS message: %w", err)
	}

	if len(dnsMsg.Question) == 0 {
		return nil, errors.New("DNS message has no questions")
	}

	q := dnsMsg.Question[0]
	qName := dns.Fqdn(q.Name)
	qType := dns.Type(q.Qtype).String()

	var ednsClientSubnet string
	if a.args.EcsClientIP != "" && a.args.EcsClientMask > 0 {
		ednsClientSubnet = fmt.Sprintf("%s/%d", a.args.EcsClientIP, a.args.EcsClientMask)
	} else {
		for _, opt := range dnsMsg.Extra {
			if edns0, ok := opt.(*dns.OPT); ok {
				for _, option := range edns0.Option {
					if ecs, ok := option.(*dns.EDNS0_SUBNET); ok {
						ednsClientSubnet = ecs.Address.String() + "/" + strconv.Itoa(int(ecs.SourceNetmask))
						break
					}
				}
			}
			if ednsClientSubnet != "" {
				break
			}
		}
	}

	ts := fmt.Sprintf("%d", time.Now().Unix())
	keyData := a.args.AccountID + a.args.AccessKeySecret + ts + qName + a.args.AccessKeyID
	keyHash := sha256.Sum256([]byte(keyData))
	keyStr := hex.EncodeToString(keyHash[:])

	url := fmt.Sprintf("http://%s/resolve?name=%s&type=%s&uid=%s&ak=%s&key=%s&ts=%s",
		a.args.ServerAddr, qName, qType, a.args.AccountID, a.args.AccessKeyID, keyStr, ts)
	if ednsClientSubnet != "" {
		url = fmt.Sprintf("%s&edns_client_subnet=%s", url, ednsClientSubnet)
	}

	a.logger.Debug("Requesting AliDNS JSON API", zap.String("url", url))
	httpReq, _ := http.NewRequestWithContext(ctx, "GET", url, nil)

	httpResp, httpErr := a.client.Do(httpReq)
	if httpErr != nil {
		a.logger.Debug("AliAPI HTTP request failed", zap.String("url", url), zap.Error(httpErr))
		return nil, fmt.Errorf("AliAPI HTTP request failed: %w", httpErr)
	}
	defer httpResp.Body.Close()

	if httpResp.StatusCode != http.StatusOK {
		body, _ := io.ReadAll(httpResp.Body)
		a.logger.Warn("AliAPI returned non-200 status code",
			zap.Int("status_code", httpResp.StatusCode),
			zap.String("body", string(body)))
		return nil, fmt.Errorf("AliAPI request failed with HTTP status %d", httpResp.StatusCode)
	}

	body, readErr := io.ReadAll(httpResp.Body)
	if readErr != nil {
		a.logger.Warn("Failed to read AliAPI response body", zap.Error(readErr))
		return nil, fmt.Errorf("failed to read AliAPI response body: %w", readErr)
	}

	a.logger.Debug("AliAPI raw response", zap.String("body", string(body)))

	var aliDNSResult DNSEntity
	jsonErr := json.Unmarshal(body, &aliDNSResult)
	if jsonErr != nil {
		a.logger.Warn("Failed to unmarshal AliAPI JSON response", zap.Error(jsonErr), zap.String("body", string(body)))
		return nil, fmt.Errorf("failed to unmarshal AliAPI JSON response: %w", jsonErr)
	}

	// === Start of logic based on AliAPI documentation ===
	responseMsg := new(dns.Msg)
	responseMsg.SetReply(dnsMsg)
	responseMsg.Authoritative = true

	// Directly use the Rcode from the API's "Status" field.
	responseMsg.SetRcode(dnsMsg, aliDNSResult.Status)
 	responseMsg.Truncated = aliDNSResult.TC
	responseMsg.RecursionAvailable = aliDNSResult.RA
	responseMsg.AuthenticatedData = aliDNSResult.AD
	responseMsg.CheckingDisabled = aliDNSResult.CD

	// Only populate the Answer section if the status is NOERROR (0).
	if aliDNSResult.Status == dns.RcodeSuccess {
		for _, ans := range aliDNSResult.Answer {
			// Keeping the original logic to only add records matching the question name.
			// if ans.Name == qName {
				record := getDNSRecord(ans)
				responseMsg.Answer = append(responseMsg.Answer, record)
			// }
		}
	} else {
		// Log DNS-level errors (like NXDOMAIN, SERVFAIL) for debugging.
		a.logger.Debug("AliAPI returned a DNS error status",
			zap.Int("status", aliDNSResult.Status),
			zap.String("remark", aliDNSResult.Remark))
	}

	// Pack the DNS message. Whether it's a success, NXDOMAIN, or SERVFAIL,
	// it's a valid DNS response that should be returned to the caller.
	packed, packErr := pool.PackBuffer(responseMsg)
	if packErr != nil {
		return nil, fmt.Errorf("failed to pack DNS response from AliAPI result: %w", packErr)
	}

	// Return the packed message and a nil error. This ensures the caller receives
	// and processes the DNS response, regardless of its Rcode.
	return packed, nil
}

// Close for AliAPIUpstream is a no-op as there are no persistent connections.
func (a *AliAPIUpstream) Close() error {
	return nil
}

// --- EventObserver placeholder for standard upstreams ---
// This is a minimal EventObserver that does nothing. It's used when we
// create standard `upstream.Upstream` instances, because the metrics
// will now be handled directly by `upstreamWrapper.ExchangeContext`.
type nopEO struct{}

func (nopEO) OnEvent(upstream.Event) {}

// upstreamWrapper wraps an upstream.Upstream and collects metrics.
type upstreamWrapper struct {
	u          upstream.Upstream
	cfg        UpstreamConfig
	metricsTag string

	mQueryTotal prometheus.Counter
	mErrorTotal prometheus.Counter
        mWinnerTotal     prometheus.Counter
	mInflight   prometheus.Gauge
        mResponseLatency prometheus.Histogram
        mConnOpened prometheus.Counter
        mConnClosed prometheus.Counter
}

func (w *upstreamWrapper) OnEvent(e upstream.Event) {
	switch e {
	case upstream.EventConnOpen:
		w.mConnOpened.Inc()
	case upstream.EventConnClose:
		w.mConnClosed.Inc()
	}
}

func newWrapper(idx int, c UpstreamConfig, metricsTag string) *upstreamWrapper {
	return &upstreamWrapper{
		cfg:        c,
		metricsTag: fmt.Sprintf("%s_%d", metricsTag, idx),
		mQueryTotal: prometheus.NewCounter(prometheus.CounterOpts{
			Name: "query_total",
			Help: "Total number of queries.",
			ConstLabels: prometheus.Labels{
				"tag":         c.Tag,
				"addr":        c.Addr,
				"metrics_tag": metricsTag,
			},
		}),
		mErrorTotal: prometheus.NewCounter(prometheus.CounterOpts{
			Name: "error_total",
			Help: "Total number of query errors.",
			ConstLabels: prometheus.Labels{
				"tag":         c.Tag,
				"addr":        c.Addr,
				"metrics_tag": metricsTag,
			},
		}),
		mWinnerTotal: prometheus.NewCounter(prometheus.CounterOpts{
			Name: "upstream_winner_total",
			Help: "Total number of times this upstream result was selected as the final response.",
			ConstLabels: prometheus.Labels{
				"tag":         c.Tag,
				"addr":        c.Addr,
				"metrics_tag": metricsTag,
			},
		}),
		mInflight: prometheus.NewGauge(prometheus.GaugeOpts{
			Name: "inflight_queries",
			Help: "Number of inflight queries.",
			ConstLabels: prometheus.Labels{
				"tag":         c.Tag,
				"addr":        c.Addr,
				"metrics_tag": metricsTag,
			},
		}),
		mResponseLatency: prometheus.NewHistogram(prometheus.HistogramOpts{
			Name:    "response_latency_millisecond",
			Help:    "Response latency in milliseconds.",
			Buckets: []float64{1, 5, 10, 20, 50, 100, 200, 500, 1000, 2000, 5000},
			ConstLabels: prometheus.Labels{
				"tag":         c.Tag,
				"addr":        c.Addr,
				"metrics_tag": metricsTag,
			},
		}),
                mConnOpened: prometheus.NewCounter(prometheus.CounterOpts{
                    Name: "conn_opened_total",
                    Help: "Total number of connections opened.",
                    ConstLabels: prometheus.Labels{
                        "tag":         c.Tag,
                        "addr":        c.Addr,
                        "metrics_tag": metricsTag,
                    },
                }),
                mConnClosed: prometheus.NewCounter(prometheus.CounterOpts{
                    Name: "conn_closed_total",
                    Help: "Total number of connections closed.",
                    ConstLabels: prometheus.Labels{
                        "tag":         c.Tag,
                        "addr":        c.Addr,
                        "metrics_tag": metricsTag,
                    },
                }),
	}
}

// ExchangeContext handles the actual exchange and updates metrics directly.
// This method takes over the responsibility of calling metric updates,
// thus `u` (the wrapped upstream) no longer needs to call them via EventObserver.
func (w *upstreamWrapper) ExchangeContext(ctx context.Context, req []byte) (*[]byte, error) {
	w.mQueryTotal.Inc()
	w.mInflight.Inc()

	start := time.Now() 

	resp, err := w.u.ExchangeContext(ctx, req) // Call the wrapped upstream's method

	w.mInflight.Dec() // Always decrement inflight after the exchange completes

	if err != nil {
		w.mErrorTotal.Inc()
	} else {
		w.mResponseLatency.Observe(float64(time.Since(start).Milliseconds()))
	}

	return resp, err
}

func (w *upstreamWrapper) Close() error {
	return w.u.Close()
}

func (w *upstreamWrapper) registerMetricsTo(r prometheus.Registerer) error {
	if err := r.Register(w.mQueryTotal); err != nil {
		return err
	}
	if err := r.Register(w.mErrorTotal); err != nil {
		return err
	}
	if err := r.Register(w.mWinnerTotal); err != nil {
		return err
	}
	if err := r.Register(w.mInflight); err != nil {
		return err
	}
	if err := r.Register(w.mResponseLatency); err != nil {
		return err
	}
            if err := r.Register(w.mConnOpened); err != nil { return err }
            if err := r.Register(w.mConnClosed); err != nil { return err }
	return nil
}

func copyPayload(p *[]byte) *[]byte {
	buf := pool.GetBuf(len(*p))
	copy(*buf, *p)
	return buf
}
