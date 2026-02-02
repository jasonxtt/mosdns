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
 * mosdns is distributed in the hope that it is useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program.  If not, see <https://www.gnu.org/licenses/>.
 */

package ip_set

import (
	"bufio"
	"compress/zlib"
	"encoding/binary"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/netip"
	"os"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/IrineSistiana/mosdns/v5/coremain"
	"github.com/IrineSistiana/mosdns/v5/pkg/matcher/netlist"
	"github.com/IrineSistiana/mosdns/v5/plugin/data_provider"
	"github.com/go-chi/chi/v5"
	"github.com/sagernet/sing/common/varbin"
	"go4.org/netipx"
)

const PluginType = "ip_set"

func init() {
	coremain.RegNewPluginFunc(PluginType, Init, func() any { return new(Args) })
}

// Args holds the configuration for ip_set plugin
type Args struct {
	IPs   []string `yaml:"ips"`
	Sets  []string `yaml:"sets"`
	Files []string `yaml:"files"`
}

// IPReceiver 定义了流式接收 IP 前缀的接口。
// 使用变长参数 ...netip.Prefix 以匹配 *netlist.List 的方法签名。
type IPReceiver interface {
	Append(...netip.Prefix)
}

var _ data_provider.IPMatcherProvider = (*IPSet)(nil)

var _ netlist.Matcher = (*IPSet)(nil)

// IPSet implements IPMatcherProvider and holds state
type IPSet struct {
	matcherVal atomic.Value

	list  *netlist.List
	files []string

	otherSets []netlist.Matcher

	mutex sync.Mutex
}

func (d *IPSet) GetIPMatcher() netlist.Matcher {
	return d
}

func (d *IPSet) Match(addr netip.Addr) bool {
	m, ok := d.matcherVal.Load().(netlist.Matcher)
	if !ok || m == nil {
		return false
	}
	return m.Match(addr)
}

// Init plugin, build IPSet and register HTTP API
func Init(bp *coremain.BP, args any) (any, error) {
	p, err := NewIPSet(bp, args.(*Args))
	if err != nil {
		return nil, err
	}
	bp.RegAPI(p.api())
	return p, nil
}

// NewIPSet creates a new IPSet
func NewIPSet(bp *coremain.BP, args *Args) (*IPSet, error) {
	p := &IPSet{files: args.Files, list: netlist.NewList()}

	// load IPs and files
	// 直接传递 p.list，由于变长参数签名匹配，它现在符合 IPReceiver 接口。
	if err := LoadFromIPsAndFiles(args.IPs, args.Files, p.list); err != nil {
		return nil, err
	}
	p.list.Sort()

	// load other sets by tag
	for _, tag := range args.Sets {
		provider, _ := bp.M().GetPlugin(tag).(data_provider.IPMatcherProvider)
		if provider == nil {
			return nil, fmt.Errorf("%s is not an IPMatcherProvider", tag)
		}
		p.otherSets = append(p.otherSets, provider.GetIPMatcher())
	}

	p.rebuildSnapshot()

	// 提示回收解析期间产生的临时对象
	go func() {
		time.Sleep(1 * time.Second)
		coremain.ManualGC()
	}()

	return p, nil
}

func (d *IPSet) rebuildSnapshot() {
	var mg MatcherGroup

	if d.list != nil && d.list.Len() > 0 {
		mg = append(mg, d.list)
	}

	if len(d.otherSets) > 0 {
		mg = append(mg, d.otherSets...)
	}

	d.matcherVal.Store(mg)
}

// api registers HTTP routes: show, save, flush, post
func (d *IPSet) api() *chi.Mux {
	r := chi.NewRouter()

	// GET /show: list in-memory prefixes
	r.Get("/show", func(w http.ResponseWriter, r *http.Request) {
		d.mutex.Lock()
		l := d.list
		d.mutex.Unlock()

		w.Header().Set("Content-Type", "text/plain; charset=utf-8")
		if l != nil {
			l.ForEach(func(pfx netip.Prefix) {
				io.WriteString(w, normalizePrefix(pfx).String()+"\n")
			})
		}
	})

	// GET /save: persist to files
	r.Get("/save", func(w http.ResponseWriter, r *http.Request) {
		d.mutex.Lock()
		defer d.mutex.Unlock()
		if err := d.saveToFiles(); err != nil {
			http.Error(w, err.Error(), http.StatusInternalServerError)
			return
		}
		w.Write([]byte("ip_set rules saved"))
	})

	// GET /flush: clear in-memory and save empty list
	r.Get("/flush", func(w http.ResponseWriter, r *http.Request) {
		d.mutex.Lock()
		defer d.mutex.Unlock()

		d.list = netlist.NewList()

		d.rebuildSnapshot()

		if err := d.saveToFiles(); err != nil {
			http.Error(w, err.Error(), http.StatusInternalServerError)
			return
		}
		w.Write([]byte("ip_set flushed and saved"))
		coremain.ManualGC()
	})

	// POST /post: replace in-memory list with provided values and save
	r.Post("/post", func(w http.ResponseWriter, r *http.Request) {
		var body struct{ Values []string `json:"values"` }
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
			http.Error(w, "invalid JSON", http.StatusBadRequest)
			return
		}

		tmpList := netlist.NewList()
		for _, s := range body.Values {
			if pfx, err := parseNetipPrefix(s); err == nil {
				tmpList.Append(pfx)
			}
		}
		tmpList.Sort()

		d.mutex.Lock()
		defer d.mutex.Unlock()

		d.list = tmpList

		d.rebuildSnapshot()

		if err := d.saveToFiles(); err != nil {
			http.Error(w, err.Error(), http.StatusInternalServerError)
			return
		}

		w.Write([]byte(fmt.Sprintf("ip_set replaced with %d entries", d.list.Len())))
		coremain.ManualGC()
	})

	return r
}

// saveToFiles writes the current list to each configured file
func (d *IPSet) saveToFiles() error {
	for _, path := range d.files {
		f, err := os.Create(path)
		if err != nil {
			return err
		}
		w := bufio.NewWriter(f)
		var writeErr error
		d.list.ForEach(func(pfx netip.Prefix) {
			if writeErr == nil {
				_, writeErr = w.WriteString(normalizePrefix(pfx).String() + "\n")
			}
		})
		if writeErr != nil {
			f.Close()
			return writeErr
		}
		if err := w.Flush(); err != nil {
			f.Close()
			return err
		}
		if err := f.Close(); err != nil {
			return err
		}
	}
	return nil
}

// LoadFromIPsAndFiles loads plain IPs and files (including .srs) into the receiver.
func LoadFromIPsAndFiles(ips, files []string, m IPReceiver) error {
	if err := loadFromIPs(ips, m); err != nil {
		return err
	}
	return loadFromFiles(files, m)
}

func parseNetipPrefix(s string) (netip.Prefix, error) {
	if strings.ContainsRune(s, '/') {
		return netip.ParsePrefix(s)
	}
	addr, err := netip.ParseAddr(s)
	if err != nil {
		return netip.Prefix{}, err
	}
	return netip.PrefixFrom(addr, addr.BitLen()), nil
}

func loadFromIPs(ips []string, m IPReceiver) error {
	for i, s := range ips {
		pfx, err := parseNetipPrefix(s)
		if err != nil {
			return fmt.Errorf("invalid ip #%d %s: %w", i, s, err)
		}
		m.Append(pfx)
	}
	return nil
}

func loadFromFiles(files []string, m IPReceiver) error {
	for i, f := range files {
		if err := loadFromFile(f, m); err != nil {
			return fmt.Errorf("failed to load file #%d %s: %w", i, f, err)
		}
	}
	return nil
}

func loadFromFile(path string, m IPReceiver) error {
	if path == "" {
		return nil
	}
	// 内存优化：流式打开文件
	f, err := os.Open(path)
	if err != nil {
		if os.IsNotExist(err) {
			fmt.Printf("[ip_set] file not found, skipping: %s\n", path)
			return nil
		}
		return err
	}
	defer f.Close()

	// 检查 SRS Magic
	var magic [3]byte
	n, _ := io.ReadFull(f, magic[:])
	if n == 3 && magic == srsMagic {
		f.Seek(0, 0)
		if ok, cnt, lastSrs := tryLoadSRS(f, m); ok {
			fmt.Printf("[ip_set] loaded %d rules from srs file: %s\n", cnt, path)
			if lastSrs != "" {
				fmt.Printf("[ip_set] last srs rule: %s\n", lastSrs)
			}
			return nil
		}
	}

	// fallback to text lines
	f.Seek(0, 0)
	// 类型断言，如果 m 是 *netlist.List，则使用其内置的高速 LoadFromReader
	if l, ok := m.(*netlist.List); ok {
		before := l.Len()
		if err := netlist.LoadFromReader(l, f); err != nil {
			return err
		}
		after := l.Len()
		fmt.Printf("[ip_set] loaded %d rules from text file: %s\n", after-before, path)
		return nil
	}

	// 兜底文本解析逻辑
	scanner := bufio.NewScanner(f)
	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		if pfx, err := parseNetipPrefix(line); err == nil {
			m.Append(pfx)
		}
	}
	return scanner.Err()
}

// MatcherGroup composes multiple netlist.Matchers
type MatcherGroup []netlist.Matcher

// Match returns true if any sub-matcher matches the address
func (mg MatcherGroup) Match(addr netip.Addr) bool {
	for _, m := range mg {
		if m.Match(addr) {
			return true
		}
	}
	return false
}

// --- SRS parsing helpers (Memory Optimized) ---
var (
	srsMagic            = [3]byte{'S', 'R', 'S'}
	ruleItemIPCIDR      = uint8(6)
	ruleItemFinal       = uint8(0xFF)
	maxSupportedVersion = uint8(3)
)

func tryLoadSRS(r io.Reader, m IPReceiver) (bool, int, string) {
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
		c, lr := readRule(br, m)
		if lr != "" {
			last = lr
		}
		count += c
	}
	return true, count, last
}

func readRule(r *bufio.Reader, m IPReceiver) (int, string) {
	ct := 0
	var last string
	mode, err := r.ReadByte()
	if err != nil {
		return 0, ""
	}
	switch mode {
	case 0:
		c, lr := readDefault(r, m)
		ct += c
		if lr != "" {
			last = lr
		}
	case 1:
		_, _ = r.ReadByte()
		n, _ := binary.ReadUvarint(r)
		for j := uint64(0); j < n; j++ {
			c, lr := readRule(r, m)
			ct += c
			if lr != "" {
				last = lr
			}
		}
		_, _ = r.ReadByte()
	}
	return ct, last
}

func readDefault(r *bufio.Reader, m IPReceiver) (int, string) {
	count := 0
	var last string
	for {
		item, err := r.ReadByte()
		if err != nil {
			break
		}
		switch item {
		case ruleItemIPCIDR:
			// 内存优化：流式解析 IPSet 范围
			err := streamParseIPSet(r, m, &count, &last)
			if err != nil {
				return count, last
			}
		case ruleItemFinal:
			return count, last
		default:
			return count, last
		}
	}
	return count, last
}

func streamParseIPSet(r *bufio.Reader, m IPReceiver, count *int, last *string) error {
	ver, err := r.ReadByte()
	if err != nil {
		return err
	}
	if ver != 1 {
		return fmt.Errorf("unsupported ipset version: %d", ver)
	}
	var length uint64
	if err := binary.Read(r, binary.BigEndian, &length); err != nil {
		return err
	}

	// 逐个处理 Range，消除原版中分配巨大切片的行为
	for i := uint64(0); i < length; i++ {
		from, err := varbin.ReadValue[[]byte](r, binary.BigEndian)
		if err != nil {
			return err
		}
		to, err := varbin.ReadValue[[]byte](r, binary.BigEndian)
		if err != nil {
			return err
		}

		fAddr, ok1 := netip.AddrFromSlice(from)
		tAddr, ok2 := netip.AddrFromSlice(to)
		if !ok1 || !ok2 {
			continue
		}

		var builder netipx.IPSetBuilder
		builder.AddRange(netipx.IPRangeFrom(fAddr, tAddr))
		ipset, _ := builder.IPSet()
		for _, pfx := range ipset.Prefixes() {
			m.Append(pfx)
			*count++
			*last = pfx.String()
		}
	}
	return nil
}

func normalizePrefix(p netip.Prefix) netip.Prefix {
	addr := p.Addr()
	if addr.Is4In6() {
		unmapped := addr.Unmap()
		bits := p.Bits() - 96
		if bits < 0 {
			bits = 0
		}
		pfx, _ := unmapped.Prefix(bits)
		return pfx
	}
	return p
}
