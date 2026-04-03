/*
 * Copyright (C) 2020-2022, IrineSistiana
 *
 * This file is part of mosdns.
 *
 * mosdns is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * mosdns is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program.  If not, see <https://www.gnu.org/licenses/>.
 */

package fastforward

import (
	"context"
	"crypto/tls"
	"errors"
	"fmt"
	"math/rand"
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

const PluginType = "forward"

func init() {
	coremain.RegNewPluginFunc(PluginType, Init, func() any { return new(Args) })
	sequence.MustRegExecQuickSetup(PluginType, quickSetup)
}

const (
	maxConcurrentQueries = 3
	queryTimeout         = time.Second * 5
)

type Args struct {
	Upstreams  []UpstreamConfig `yaml:"upstreams"`
	Concurrent int              `yaml:"concurrent"`

	// Global options.
	Socks5       string `yaml:"socks5"`
	SoMark       int    `yaml:"so_mark"`
	BindToDevice string `yaml:"bind_to_device"`
	Bootstrap    string `yaml:"bootstrap"`
	BootstrapVer int    `yaml:"bootstrap_version"`
}

type UpstreamConfig struct {
	Tag                  string `yaml:"tag"`
	Addr                 string `yaml:"addr"` // Required.
	DialAddr             string `yaml:"dial_addr"`
	IdleTimeout          int    `yaml:"idle_timeout"`
	UpstreamQueryTimeout int    `yaml:"upstream_query_timeout"` // New option for upstream timeout.

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
	f, err := NewForward(args.(*Args), Opts{Logger: bp.L(), MetricsTag: bp.Tag()})
	if err != nil {
		return nil, err
	}
	if err := f.RegisterMetricsTo(prometheus.WrapRegistererWithPrefix(PluginType+"_", bp.M().GetMetricsReg())); err != nil {
		_ = f.Close()
		return nil, err
	}
	return f, nil
}

var _ sequence.Executable = (*Forward)(nil)
var _ sequence.QuickConfigurableExec = (*Forward)(nil)

type Forward struct {
	args *Args

	logger       *zap.Logger
	us           []*upstreamWrapper
	tag2Upstream map[string]*upstreamWrapper // for fast tag lookup only.
}

type Opts struct {
	Logger     *zap.Logger
	MetricsTag string
}

// NewForward inits a Forward from given args.
// args must contain at least one upstream.
func NewForward(args *Args, opt Opts) (*Forward, error) {
	if len(args.Upstreams) == 0 {
		return nil, errors.New("no upstream is configured")
	}
	if opt.Logger == nil {
		opt.Logger = zap.NewNop()
	}

	f := &Forward{
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
	}

	for i, c := range args.Upstreams {
		if len(c.Addr) == 0 {
			return nil, fmt.Errorf("#%d upstream invalid args, addr is required", i)
		}
		applyGlobal(&c)

		uw := newWrapper(i, c, opt.MetricsTag)
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

		u, err := upstream.NewUpstream(c.Addr, uOpt)
		if err != nil {
			_ = f.Close()
			return nil, fmt.Errorf("failed to init upstream #%d: %w", i, err)
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

func (f *Forward) RegisterMetricsTo(r prometheus.Registerer) error {
	for _, wu := range f.us {
		// Only register metrics for upstream that has a tag.
		if len(wu.cfg.Tag) == 0 {
			continue
		}
		if err := wu.registerMetricsTo(r); err != nil {
			return err
		}
	}
	return nil
}

func (f *Forward) Exec(ctx context.Context, qCtx *query_context.Context) (err error) {
	r, err := f.exchange(ctx, qCtx, f.us)
	if err != nil {
		return err
	}
	qCtx.SetResponse(r)
	return nil
}

// QuickConfigureExec format: [upstream_tag]...
func (f *Forward) QuickConfigureExec(args string) (any, error) {
	var us []*upstreamWrapper
	if len(args) == 0 { // No args, use all upstreams.
		us = f.us
	} else { // Pick up upstreams by tags.
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

func (f *Forward) Close() error {
	for _, u := range f.us {
		_ = u.Close()
	}
	return nil
}

// ===============================================================================
// ===== VVVV  The only modified function is `exchange` below. VVVV =====
// ===============================================================================

func (f *Forward) exchange(ctx context.Context, qCtx *query_context.Context, us []*upstreamWrapper) (*dns.Msg, error) {
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

	type res struct {
		r   *dns.Msg
		err error
		u   *upstreamWrapper
	}

	resChan := make(chan res)
	done := make(chan struct{})
	defer close(done)

	// --- MODIFICATION START ---
	// Variables to store the best available "fallback" results according to priority.
	var lastSuccessOrNXRes *dns.Msg // Priority 2: Stores NOERROR or NXDOMAIN responses.
	var lastSuccessOrNXUpstream *upstreamWrapper
	var lastOtherRes *dns.Msg // Priority 3: Stores other responses like SERVFAIL.
	var lastOtherUpstream *upstreamWrapper
	var lastError error // Priority 4: Stores the first encountered network error.
	// --- MODIFICATION END ---

	storeSelectedUpstream := func(u *upstreamWrapper) {
		if u != nil {
			qCtx.StoreValue(query_context.KeySelectedUpstream, u.name())
		}
	}

	r := rand.Intn(len(us))
	for i := 0; i < concurrent; i++ {
		u := us[(r+i)%len(us)]
		qc := copyPayload(queryPayload)

		upstreamTimeout := time.Duration(u.cfg.UpstreamQueryTimeout) * time.Millisecond
		if upstreamTimeout == 0 {
			upstreamTimeout = queryTimeout
		}

		go func(uqid uint32, question dns.Question) {
			defer pool.ReleaseBuf(qc)
			upstreamCtx, cancel := context.WithTimeout(context.Background(), upstreamTimeout)
			defer cancel()

			var r *dns.Msg
			respPayload, err := u.ExchangeContext(upstreamCtx, *qc)
			if err != nil {
				// Skip logging "context deadline exceeded"
			} else {
				r = new(dns.Msg)
				err = r.Unpack(*respPayload)
				pool.ReleaseBuf(respPayload)
				if err != nil {
					r = nil
				}
			}
			select {
			case resChan <- res{r: r, err: err, u: u}:
			case <-done:
			}
		}(qCtx.Id(), qCtx.QQuestion())
	}

	for i := 0; i < concurrent; i++ {
		select {
		case res := <-resChan:
			r, err, u := res.r, res.err, res.u

			// --- MODIFICATION START ---
			if err != nil {
				if lastError == nil { // Record the first network error encountered.
					lastError = err
				}
				continue // Move to the next result.
			}

			// Priority 1: A response with an IP address is always the best. Return immediately.
			if len(r.Answer) > 0 {
				for _, ans := range r.Answer {
					if a, ok := ans.(*dns.A); ok && len(a.A) > 0 {
						storeSelectedUpstream(u)
						return r, nil
					}
					if aaaa, ok := ans.(*dns.AAAA); ok && len(aaaa.AAAA) > 0 {
						storeSelectedUpstream(u)
						return r, nil
					}
				}
			}

			// If no IP, classify and store other valid responses for later decision.
			// Priority 2: A definitive response (NOERROR or NXDOMAIN).
			if r.Rcode == dns.RcodeSuccess || r.Rcode == dns.RcodeNameError {
				if lastSuccessOrNXRes == nil {
					lastSuccessOrNXRes = r
					lastSuccessOrNXUpstream = u
				}
			} else { // Priority 3: Other responses like SERVFAIL, REFUSED, etc.
				if lastOtherRes == nil {
					lastOtherRes = r
					lastOtherUpstream = u
				}
			}
			// --- MODIFICATION END ---

		case <-ctx.Done():
			return nil, context.Cause(ctx)
		}
	}

	// --- MODIFICATION START ---
	// After all concurrent queries are done, return the best result we found based on priority.
	if lastSuccessOrNXRes != nil {
		storeSelectedUpstream(lastSuccessOrNXUpstream)
		return lastSuccessOrNXRes, nil
	}
	if lastOtherRes != nil {
		storeSelectedUpstream(lastOtherUpstream)
		return lastOtherRes, nil
	}
	if lastError != nil {
		// Priority 4: If all we got were network errors, propagate the first error up.
		return nil, lastError
	}

	// Fallback: This case should be rare but is more informative than returning `nil, nil`.
	return nil, errors.New("all upstreams failed or returned no usable response")
	// --- MODIFICATION END ---
}

// ===============================================================================
// ===== ^^^^ The only modified function is `exchange` above. ^^^^ =====
// ===============================================================================

func quickSetup(bq sequence.BQ, s string) (any, error) {
	args := new(Args)
	args.Concurrent = maxConcurrentQueries
	for _, u := range strings.Fields(s) {
		args.Upstreams = append(args.Upstreams, UpstreamConfig{Addr: u})
	}
	return NewForward(args, Opts{Logger: bq.L()})
}
