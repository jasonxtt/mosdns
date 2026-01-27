package domain_set_light

import (
	"bufio"
	"bytes"
	"compress/zlib"
	"encoding/binary"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"sync"

	"github.com/IrineSistiana/mosdns/v5/coremain"
	"github.com/IrineSistiana/mosdns/v5/pkg/matcher/domain"
	"github.com/IrineSistiana/mosdns/v5/plugin/data_provider"
	"github.com/go-chi/chi/v5"
	scdomain "github.com/sagernet/sing/common/domain"
	"github.com/sagernet/sing/common/varbin"
)

// [修改] 插件类型名称
const PluginType = "domain_set_light"

func init() {
	coremain.RegNewPluginFunc(PluginType, Init, func() any { return new(Args) })
}

type Args struct {
	Exps  []string `yaml:"exps"`
	Sets  []string `yaml:"sets"` // 保留字段以防配置文件报错，但内部不再加载
	Files []string `yaml:"files"`
}

type domainPayload struct {
	Values []string `json:"values"`
}

// 接口实现检查
var _ data_provider.DomainMatcherProvider = (*DomainSetLight)(nil)
var _ domain.Matcher[struct{}] = (*DomainSetLight)(nil)
var _ data_provider.RuleExporter = (*DomainSetLight)(nil)

// 定义一个简单的接口，用于复用 SRS 解析逻辑（解耦 Trie 树依赖）
type ruleAdder interface {
	Add(string, struct{}) error
}

// 字符串收集器，用于替代 MixMatcher 接收解析出来的规则
type stringCollector struct {
	rules *[]string
}

func (c *stringCollector) Add(s string, _ struct{}) error {
	*c.rules = append(*c.rules, s)
	return nil
}

type DomainSetLight struct {
	mu sync.RWMutex
	// [优化] 移除了 heavy 的 mixM 和 otherM
	// mixM   *domain.MixMatcher[struct{}]
	// otherM []domain.Matcher[struct{}]

	ruleFile string
	rules    []string // 仅维护字符串列表，内存占用极低

	// 新增：订阅者列表
	subscribers []func()
}

// GetRules 实现 RuleExporter 接口
func (d *DomainSetLight) GetRules() ([]string, error) {
	d.mu.RLock()
	defer d.mu.RUnlock()
	// 返回规则的副本
	rulesCopy := make([]string, len(d.rules))
	copy(rulesCopy, d.rules)
	return rulesCopy, nil
}

// Subscribe 实现 RuleExporter 接口
func (d *DomainSetLight) Subscribe(cb func()) {
	d.mu.Lock()
	defer d.mu.Unlock()
	d.subscribers = append(d.subscribers, cb)
}

// notifySubscribers 通知所有订阅者
func (d *DomainSetLight) notifySubscribers() {
	d.mu.RLock()
	subs := make([]func(), len(d.subscribers))
	copy(subs, d.subscribers)
	d.mu.RUnlock()

	for _, cb := range subs {
		go cb()
	}
}

// initAndLoadRules 加载规则到字符串切片
func (d *DomainSetLight) initAndLoadRules(exps, files []string) ([]string, error) {
	allRules := make([]string, 0, len(exps)+len(files)*100)

	// Load from expressions
	allRules = append(allRules, exps...)

	// Load from files
	for i, f := range files {
		rules, err := d.loadFileInternal(f)
		if err != nil {
			return nil, fmt.Errorf("failed to load file %d %s: %w", i, f, err)
		}
		allRules = append(allRules, rules...)
	}

	return allRules, nil
}

// loadFileInternal 读取文件内容并解析为规则字符串
func (d *DomainSetLight) loadFileInternal(f string) ([]string, error) {
	if f == "" {
		return nil, nil
	}
	b, err := os.ReadFile(f)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, err
	}

	// 1. 尝试作为 SRS 解析
	var srsRules []string
	collector := &stringCollector{rules: &srsRules}
	if ok, count, last := tryLoadSRS(b, collector); ok {
		fmt.Printf("[%s] loaded %d rules from srs file: %s (last rule: %s)\n", PluginType, count, f, last)
		return srsRules, nil
	}

	// 2. 作为普通文本解析
	var rules []string
	var lastTxt string
	scanner := bufio.NewScanner(bytes.NewReader(b))
	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		// [优化] 直接存入字符串，无需 MixMatcher.Add 的开销
		rules = append(rules, line)
		lastTxt = line
	}

	if len(rules) > 0 {
		fmt.Printf("[%s] loaded %d rules from text file: %s (last rule: %s)\n", PluginType, len(rules), f, lastTxt)
	}
	return rules, scanner.Err()
}

func Init(bp *coremain.BP, args any) (any, error) {
	cfg := args.(*Args)
	ds := &DomainSetLight{
		subscribers: make([]func(), 0),
	}

	if len(cfg.Files) > 0 {
		ds.ruleFile = cfg.Files[0]
	}

	// 使用新的加载逻辑
	loadedRules, err := ds.initAndLoadRules(cfg.Exps, cfg.Files)
	if err != nil {
		return nil, fmt.Errorf("failed to load rules: %w", err)
	}
	ds.rules = loadedRules

	// [注意] 这里故意忽略了 cfg.Sets 的处理
	// 因为本插件不负责匹配，不需要持有其他插件的引用

	bp.RegAPI(ds.api())
	return ds, nil
}

func (d *DomainSetLight) GetDomainMatcher() domain.Matcher[struct{}] {
	return d
}

// Match [重要修改] 恒定返回 false，不占用 CPU，不查找 Trie
func (d *DomainSetLight) Match(domainStr string) (value struct{}, ok bool) {
	return struct{}{}, false
}

// ================== API FUNCTION (CORRECTED) ==================

func (d *DomainSetLight) api() *chi.Mux {
	r := chi.NewRouter()

	r.Get("/show", coremain.WithAsyncGC(func(w http.ResponseWriter, r *http.Request) {
		d.mu.RLock()
		defer d.mu.RUnlock()
		w.Header().Set("Content-Type", "text/plain; charset=utf-8")
		for _, rule := range d.rules {
			fmt.Fprintln(w, rule)
		}
	}))

	r.Get("/save", coremain.WithAsyncGC(func(w http.ResponseWriter, r *http.Request) {
		d.mu.RLock()
		defer d.mu.RUnlock()
		if d.ruleFile == "" {
			http.Error(w, "no file configured", http.StatusInternalServerError)
			return
		}
		if err := writeRulesToFile(d.ruleFile, d.rules); err != nil {
			http.Error(w, err.Error(), http.StatusInternalServerError)
			return
		}
		w.WriteHeader(http.StatusOK)
	}))

	r.Post("/post", coremain.WithAsyncGC(func(w http.ResponseWriter, r *http.Request) {
		var p domainPayload
		if err := json.NewDecoder(r.Body).Decode(&p); err != nil {
			http.Error(w, "invalid JSON", http.StatusBadRequest)
			return
		}

		if d.ruleFile == "" || !strings.EqualFold(filepath.Ext(d.ruleFile), ".txt") {
			http.Error(w, "no txt file configured, cannot post", http.StatusBadRequest)
			return
		}

		// [优化] 直接替换 slice，无需重建 Trie
		d.mu.Lock()
		d.rules = p.Values
		d.mu.Unlock()

		if err := writeRulesToFile(d.ruleFile, d.rules); err != nil {
			http.Error(w, err.Error(), http.StatusInternalServerError)
			return
		}
		
		// 规则更新成功，通知订阅者 (domain_mapper)
		d.notifySubscribers()

		w.WriteHeader(http.StatusOK)
		fmt.Fprintf(w, "domain_set_light replaced with %d entries", len(d.rules))
	}))

	return r
}

// ==============================================================

func writeRulesToFile(path string, rules []string) error {
	f, err := os.Create(path)
	if err != nil {
		return err
	}
	defer f.Close()
	writer := bufio.NewWriter(f)
	for _, r := range rules {
		if _, err := writer.WriteString(r + "\n"); err != nil {
			return err
		}
	}
	return writer.Flush()
}

// --- SRS 解析函数 (修改为适配 ruleAdder 接口) ---

func tryLoadSRS(b []byte, m ruleAdder) (bool, int, string) {
	r := bytes.NewReader(b)
	var mb [3]byte
	if _, err := io.ReadFull(r, mb[:]); err != nil || mb != magicBytes {
		return false, 0, ""
	}
	var version uint8
	if err := binary.Read(r, binary.BigEndian, &version); err != nil || version > ruleSetVersionCurrent {
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
	var lastRule string
	for i := uint64(0); i < length; i++ {
		count += readRuleCompat(br, m, &lastRule)
	}
	return true, count, lastRule
}

var (
	magicBytes            = [3]byte{0x53, 0x52, 0x53}
	ruleItemDomain        = uint8(2)
	ruleItemDomainKeyword = uint8(3)
	ruleItemDomainRegex   = uint8(4)
	ruleItemFinal         = uint8(0xFF)
)

const ruleSetVersionCurrent = 3

// 修改签名接收 ruleAdder
func readRuleCompat(r *bufio.Reader, m ruleAdder, last *string) int {
	ct := 0
	mode, err := r.ReadByte()
	if err != nil {
		return 0
	}
	switch mode {
	case 0:
		ct += readDefaultRuleCompat(r, m, last)
	case 1:
		r.ReadByte()
		n, _ := binary.ReadUvarint(r)
		for i := uint64(0); i < n; i++ {
			ct += readRuleCompat(r, m, last)
		}
		r.ReadByte()
	}
	return ct
}

// 修改签名接收 ruleAdder
func readDefaultRuleCompat(r *bufio.Reader, m ruleAdder, last *string) int {
	count := 0
	for {
		item, err := r.ReadByte()
		if err != nil {
			break
		}
		switch item {
		case ruleItemDomain:
			matcher, err := scdomain.ReadMatcher(r)
			if err != nil {
				return count
			}
			doms, suffix := matcher.Dump()
			for _, d := range doms {
				*last = "full:" + d
				if m.Add(*last, struct{}{}) == nil {
					count++
				}
			}
			for _, d := range suffix {
				*last = "domain:" + d
				if m.Add(*last, struct{}{}) == nil {
					count++
				}
			}
		case ruleItemDomainKeyword:
			sl, _ := varbin.ReadValue[[]string](r, binary.BigEndian)
			for _, d := range sl {
				*last = "keyword:" + d
				if m.Add(*last, struct{}{}) == nil {
					count++
				}
			}
		case ruleItemDomainRegex:
			sl, _ := varbin.ReadValue[[]string](r, binary.BigEndian)
			for _, d := range sl {
				*last = "regexp:" + d
				if m.Add(*last, struct{}{}) == nil {
					count++
				}
			}
		case ruleItemFinal:
			return count
		default:
			return count
		}
	}
	return count
}
