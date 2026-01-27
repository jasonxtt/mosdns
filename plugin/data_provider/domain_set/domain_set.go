package domain_set

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

const PluginType = "domain_set"

func init() {
	coremain.RegNewPluginFunc(PluginType, Init, func() any { return new(Args) })
}

type Args struct {
	Exps  []string `yaml:"exps"`
	Sets  []string `yaml:"sets"`
	Files []string `yaml:"files"`
}

type domainPayload struct {
	Values []string `json:"values"`
}

var _ data_provider.DomainMatcherProvider = (*DomainSet)(nil)
var _ domain.Matcher[struct{}] = (*DomainSet)(nil)
// 确保实现了 RuleExporter 接口
var _ data_provider.RuleExporter = (*DomainSet)(nil)

type DomainSet struct {
	mu     sync.RWMutex
	mixM   *domain.MixMatcher[struct{}]
	otherM []domain.Matcher[struct{}]

	ruleFile string
	rules    []string

	// 新增：订阅者列表
	subscribers []func()
}

// GetRules 实现 RuleExporter 接口
func (d *DomainSet) GetRules() ([]string, error) {
	d.mu.RLock()
	defer d.mu.RUnlock()
	// 返回规则的副本，防止外部修改
	rulesCopy := make([]string, len(d.rules))
	copy(rulesCopy, d.rules)
	return rulesCopy, nil
}

// Subscribe 实现 RuleExporter 接口
func (d *DomainSet) Subscribe(cb func()) {
	d.mu.Lock()
	defer d.mu.Unlock()
	d.subscribers = append(d.subscribers, cb)
}

// notifySubscribers 通知所有订阅者（异步执行）
func (d *DomainSet) notifySubscribers() {
	d.mu.RLock()
	subs := make([]func(), len(d.subscribers))
	copy(subs, d.subscribers)
	d.mu.RUnlock()

	for _, cb := range subs {
		go cb()
	}
}

// initAndLoadRules is a new internal function for loading rules within this plugin.
// It populates the matcher and returns the list of rule strings.
func (d *DomainSet) initAndLoadRules(exps, files []string) ([]string, error) {
	allRules := make([]string, 0, len(exps)+len(files)*100)

	// Load from expressions
	if err := LoadExps(exps, d.mixM); err != nil {
		return nil, err
	}
	allRules = append(allRules, exps...)

	// Load from files
	for i, f := range files {
		// Use a new internal loading function for files
		rules, err := d.loadFileInternal(f)
		if err != nil {
			return nil, fmt.Errorf("failed to load file %d %s: %w", i, f, err)
		}
		allRules = append(allRules, rules...)
	}

	return allRules, nil
}

// loadFileInternal is the new internal version of LoadFile.
// It loads rules into the instance's mixM and returns the rule strings.
func (d *DomainSet) loadFileInternal(f string) ([]string, error) {
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

	if ok, count, last := tryLoadSRS(b, d.mixM); ok {
		fmt.Printf("[domain_set] loaded %d rules from srs file: %s (last rule: %s)\n", count, f, last)
		return nil, nil
	}

	var rules []string
	var lastTxt string
	before := d.mixM.Len()
	scanner := bufio.NewScanner(bytes.NewReader(b))
	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		if err := d.mixM.Add(line, struct{}{}); err == nil {
			rules = append(rules, line)
			lastTxt = line
		}
	}

	after := d.mixM.Len()
	if after > before {
		fmt.Printf("[domain_set] loaded %d rules from text file: %s (last rule: %s)\n", after-before, f, lastTxt)
	}
	return rules, scanner.Err()
}

func Init(bp *coremain.BP, args any) (any, error) {
	cfg := args.(*Args)
	ds := &DomainSet{
		mixM:        domain.NewDomainMixMatcher(),
		otherM:      make([]domain.Matcher[struct{}], 0, len(cfg.Sets)),
		subscribers: make([]func(), 0), // 初始化订阅者列表
	}

	if len(cfg.Files) > 0 {
		ds.ruleFile = cfg.Files[0]
	}

	// Use the new internal loading function to avoid changing public API.
	loadedRules, err := ds.initAndLoadRules(cfg.Exps, cfg.Files)
	if err != nil {
		return nil, fmt.Errorf("failed to load rules: %w", err)
	}
	ds.rules = loadedRules
                coremain.ManualGC()

	for _, tag := range cfg.Sets {
		provider, ok := bp.M().GetPlugin(tag).(data_provider.DomainMatcherProvider)
		if !ok || provider == nil {
			return nil, fmt.Errorf("%s is not a DomainMatcherProvider", tag)
		}
		ds.otherM = append(ds.otherM, provider.GetDomainMatcher())
	}

	bp.RegAPI(ds.api())
	return ds, nil
}

func (d *DomainSet) GetDomainMatcher() domain.Matcher[struct{}] {
	return d
}

func (d *DomainSet) Match(domainStr string) (value struct{}, ok bool) {
	d.mu.RLock()
	m := d.mixM
	d.mu.RUnlock()

	if _, ok := m.Match(domainStr); ok {
		return struct{}{}, true
	}

	for _, matcher := range d.otherM {
		if _, ok := matcher.Match(domainStr); ok {
			return struct{}{}, true
		}
	}

	return struct{}{}, false
}

func (d *DomainSet) api() *chi.Mux {
	r := chi.NewRouter()

	r.Get("/show", func(w http.ResponseWriter, r *http.Request) {
		d.mu.RLock()
		defer d.mu.RUnlock()
		w.Header().Set("Content-Type", "text/plain; charset=utf-8")
		for _, rule := range d.rules {
			fmt.Fprintln(w, rule)
		}
	})

	r.Get("/save", func(w http.ResponseWriter, r *http.Request) {
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
	})

	r.Post("/post", func(w http.ResponseWriter, r *http.Request) {
		var p domainPayload
		if err := json.NewDecoder(r.Body).Decode(&p); err != nil {
			http.Error(w, "invalid JSON", http.StatusBadRequest)
			return
		}

		if d.ruleFile == "" || !strings.EqualFold(filepath.Ext(d.ruleFile), ".txt") {
			http.Error(w, "no txt file configured, cannot post", http.StatusBadRequest)
			return
		}

		tmpMix := domain.NewDomainMixMatcher()
		tmpRules := make([]string, 0, len(p.Values))
		for _, pat := range p.Values {
			if err := tmpMix.Add(pat, struct{}{}); err == nil {
				tmpRules = append(tmpRules, pat)
			}
		}

		d.mu.Lock()
		d.mixM = tmpMix
		d.rules = tmpRules
		d.mu.Unlock()

        tmpMix = nil
        tmpRules = nil

		if err := writeRulesToFile(d.ruleFile, d.rules); err != nil {
			http.Error(w, err.Error(), http.StatusInternalServerError)
			return
		}
		
		// 规则更新成功，通知订阅者
		d.notifySubscribers()

        coremain.ManualGC()

		w.WriteHeader(http.StatusOK)
		fmt.Fprintf(w, "domain_set replaced with %d entries", len(d.rules))
	})

	return r
}

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

// --- Public loading functions (UNCHANGED to maintain compatibility) ---

func LoadExpsAndFiles(exps, fs []string, m *domain.MixMatcher[struct{}]) error {
	if err := LoadExps(exps, m); err != nil {
		return err
	}
	return LoadFiles(fs, m)
}

func LoadExps(exps []string, m *domain.MixMatcher[struct{}]) error {
	for i, exp := range exps {
		if err := m.Add(exp, struct{}{}); err != nil {
			return fmt.Errorf("failed to load exp %d %s: %w", i, exp, err)
		}
	}
	return nil
}

func LoadFiles(fs []string, m *domain.MixMatcher[struct{}]) error {
	for i, f := range fs {
		if err := LoadFile(f, m); err != nil {
			return fmt.Errorf("failed to load file %d %s: %w", i, f, err)
		}
	}
	return nil
}

func LoadFile(f string, m *domain.MixMatcher[struct{}]) error {
	if f == "" {
		return nil
	}
	b, err := os.ReadFile(f)
	if err != nil {
		if os.IsNotExist(err) {
			return nil
		}
		return err
	}

	if ok, count, last := tryLoadSRS(b, m); ok {
		fmt.Printf("[domain_set] loaded %d rules from srs file: %s (last rule: %s)\n", count, f, last)
		return nil
	}

	var lastTxt string
	before := m.Len()
	scanner := bufio.NewScanner(bytes.NewReader(b))
	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		lastTxt = line
		m.Add(line, struct{}{}) // Ignore error to match original behavior
	}

	after := m.Len()
	if after > before {
		fmt.Printf("[domain_set] loaded %d rules from text file: %s (last rule: %s)\n", after-before, f, lastTxt)
	}
	return scanner.Err()
}

// --- SRS parsing functions (mostly unchanged) ---

func tryLoadSRS(b []byte, m *domain.MixMatcher[struct{}]) (bool, int, string) {
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

func readRuleCompat(r *bufio.Reader, m *domain.MixMatcher[struct{}], last *string) int {
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

func readDefaultRuleCompat(r *bufio.Reader, m *domain.MixMatcher[struct{}], last *string) int {
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
