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
	"github.com/IrineSistiana/mosdns/v5/mlog"
	"github.com/IrineSistiana/mosdns/v5/pkg/matcher/domain"
	"github.com/IrineSistiana/mosdns/v5/plugin/data_provider"
	"github.com/IrineSistiana/mosdns/v5/plugin/data_provider/matcher_adapter"
	"github.com/go-chi/chi/v5"
	scdomain "github.com/sagernet/sing/common/domain"
	"github.com/sagernet/sing/common/varbin"
	"go.uber.org/zap"
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

// rustMatcher 是 Rust domain matcher 后端的接口抽象，非 tagged 构建时编译为 nil 占位。
// RustMatcher is the public interface for a Rust-backed domain matcher.
type RustMatcher interface {
	Match(string) (bool, error)
	Close() error
}

var rustDomainMatcherBuilder = buildRustDomainMatcher

type DomainSet struct {
	mu       sync.RWMutex
	updateMu sync.Mutex
	mixM     *domain.MixMatcher[struct{}]
	otherM   []domain.Matcher[struct{}]

	ruleFile  string
	rules     []string
	rustRules []string

	// 新增：订阅者列表
	subscribers []func()

	// 实验性 Rust 后端（仅当 MOSDNS_MATCHER_BACKEND=rust 且 linux+cgo 时非 nil）
	rustMatcher RustMatcher
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

// BuildRustDomainMatcher builds a Rust snapshot when the experimental backend
// is enabled. A disabled backend returns (nil, nil); an enabled backend
// returns an error without publishing a handle when the build fails.
func BuildRustDomainMatcher(rules []string) (RustMatcher, error) {
	return rustDomainMatcherBuilder(rules)
}

// InitRustDomainMatcher creates a Rust domain matcher when the experimental
// backend is enabled. Build errors use the established Go fallback.
func InitRustDomainMatcher(rules []string) RustMatcher {
	rb, err := BuildRustDomainMatcher(rules)
	if err != nil {
		fmt.Printf("[domain_set] failed to initialize experimental rust matcher: %v\n", err)
	}
	return rb
}

// notifySubscribers 通知所有订阅者（异步执行）
// Close implements io.Closer for lifecycle cleanup of the Rust handle.
func (d *DomainSet) Close() error {
	d.updateMu.Lock()
	defer d.updateMu.Unlock()

	d.mu.Lock()
	old := d.rustMatcher
	d.rustMatcher = nil
	d.mu.Unlock()
	if old != nil {
		return old.Close()
	}
	return nil
}

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
func (d *DomainSet) initAndLoadRules(exps, files []string) ([]string, []string, error) {
	allRules := make([]string, 0, len(exps)+len(files)*100)
	rustRules := make([]string, 0, len(exps)+len(files)*100)

	// Load from expressions
	if err := LoadExps(exps, d.mixM); err != nil {
		return nil, nil, err
	}
	allRules = append(allRules, exps...)
	rustRules = append(rustRules, exps...)

	// Load from files
	for i, f := range files {
		rules, err := d.loadFileInternalWithRules(f, &rustRules)
		if err != nil {
			return nil, nil, fmt.Errorf("failed to load file %d %s: %w", i, f, err)
		}
		allRules = append(allRules, rules...)
	}

	return allRules, rustRules, nil
}

// loadFileInternal is the new internal version of LoadFile.
// It loads rules into the instance's mixM and returns the rule strings.
func (d *DomainSet) loadFileInternal(f string) ([]string, error) {
	return d.loadFileInternalWithRules(f, nil)
}

func (d *DomainSet) loadFileInternalWithRules(f string, rustRules *[]string) ([]string, error) {
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

	if ok, count, last := tryLoadSRSWithRules(b, d.mixM, rustRules); ok {
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
			if rustRules != nil {
				*rustRules = append(*rustRules, line)
			}
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
	loadedRules, rustRules, err := ds.initAndLoadRules(cfg.Exps, cfg.Files)
	if err != nil {
		return nil, fmt.Errorf("failed to load rules: %w", err)
	}
	ds.rules = loadedRules
	ds.rustRules = rustRules
	coremain.ManualGC()
	rb := InitRustDomainMatcher(ds.rustRules)
	ds.mu.Lock()
	ds.rustMatcher = rb
	ds.mu.Unlock()
	if rb != nil {
		fmt.Printf("[domain_set] experimental rust matcher enabled (%d rules)\n", len(ds.rustRules))
	}

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
	// Keep the Rust and Go snapshots paired while matching. A POST waits for
	// this read lock before publishing the next generation.
	d.mu.RLock()
	rb := d.rustMatcher
	var rustErr error
	if rb != nil && matcher_adapter.RustDomainInputSupported(domainStr) {
		var matched bool
		matched, rustErr = rb.Match(domainStr)
		if rustErr == nil && matched {
			d.mu.RUnlock()
			return struct{}{}, true
		}
	}

	m := d.mixM
	goMatched := m != nil
	if goMatched {
		_, goMatched = m.Match(domainStr)
	}
	d.mu.RUnlock()

	if rustErr != nil {
		// Circuit breaker: only close if this exact backend is still active.
		// The generation check prevents a stale reader from closing a new POST.
		closeRust := false
		d.mu.Lock()
		if d.rustMatcher == rb {
			d.rustMatcher = nil
			closeRust = true
		}
		d.mu.Unlock()
		if closeRust {
			_ = rb.Close()
		}
	}
	if goMatched {
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
		d.updateMu.Lock()
		defer d.updateMu.Unlock()
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
		d.updateMu.Lock()
		defer d.updateMu.Unlock()

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

		// Build the request's immutable Rust candidate before publishing any
		// part of the new generation. A Rust-only failure keeps the Go
		// candidate eligible for publication.
		rb, err := BuildRustDomainMatcher(tmpRules)
		if err != nil {
			mlog.L().Warn("domain_set Rust matcher build failed; using Go-only generation",
				zap.Error(err), zap.Int("rules", len(tmpRules)))
			if rb != nil {
				_ = rb.Close()
				rb = nil
			}
		}

		if err := writeRulesToFile(d.ruleFile, tmpRules); err != nil {
			http.Error(w, err.Error(), http.StatusInternalServerError)
			if rb != nil {
				_ = rb.Close()
			}
			return
		}

		d.mu.Lock()
		oldRust := d.rustMatcher
		d.mixM = tmpMix
		d.rules = tmpRules
		d.rustRules = tmpRules
		d.rustMatcher = rb
		d.mu.Unlock()
		if oldRust != nil {
			_ = oldRust.Close()
		}

		// 规则更新成功，通知订阅者
		d.notifySubscribers()

		coremain.ManualGC()

		w.WriteHeader(http.StatusOK)
		fmt.Fprintf(w, "domain_set replaced with %d entries", len(tmpRules))
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
	_, err := LoadExpsAndFilesWithRules(exps, fs, m)
	return err
}

// LoadExpsAndFilesWithRules loads the Go matcher and returns the exact
// accepted rule batch for a Rust candidate. SRS entries are converted to the
// equivalent typed rules without changing LoadFile's public text behavior.
func LoadExpsAndFilesWithRules(exps, fs []string, m *domain.MixMatcher[struct{}]) ([]string, error) {
	rules := make([]string, 0, len(exps)+len(fs)*100)
	if err := LoadExps(exps, m); err != nil {
		return nil, err
	}
	rules = append(rules, exps...)
	for i, f := range fs {
		loaded, err := loadFileWithRules(f, m)
		if err != nil {
			return nil, fmt.Errorf("failed to load file %d %s: %w", i, f, err)
		}
		rules = append(rules, loaded...)
	}
	return rules, nil
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
	_, err := loadFileWithRules(f, m)
	return err
}

func loadFileWithRules(f string, m *domain.MixMatcher[struct{}]) ([]string, error) {
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

	var rustRules []string
	if ok, count, last := tryLoadSRSWithRules(b, m, &rustRules); ok {
		fmt.Printf("[domain_set] loaded %d rules from srs file: %s (last rule: %s)\n", count, f, last)
		return rustRules, nil
	}

	var rules []string
	var lastTxt string
	before := m.Len()
	scanner := bufio.NewScanner(bytes.NewReader(b))
	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		lastTxt = line
		if err := m.Add(line, struct{}{}); err == nil {
			rules = append(rules, line)
		}
	}

	after := m.Len()
	if after > before {
		fmt.Printf("[domain_set] loaded %d rules from text file: %s (last rule: %s)\n", after-before, f, lastTxt)
	}
	return rules, scanner.Err()
}

// --- SRS parsing functions (mostly unchanged) ---

func tryLoadSRS(b []byte, m *domain.MixMatcher[struct{}]) (bool, int, string) {
	return tryLoadSRSWithRules(b, m, nil)
}

func tryLoadSRSWithRules(b []byte, m *domain.MixMatcher[struct{}], rustRules *[]string) (bool, int, string) {
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
		count += readRuleCompatWithRules(br, m, &lastRule, rustRules)
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
	return readRuleCompatWithRules(r, m, last, nil)
}

func readRuleCompatWithRules(r *bufio.Reader, m *domain.MixMatcher[struct{}], last *string, rustRules *[]string) int {
	ct := 0
	mode, err := r.ReadByte()
	if err != nil {
		return 0
	}
	switch mode {
	case 0:
		ct += readDefaultRuleCompatWithRules(r, m, last, rustRules)
	case 1:
		r.ReadByte()
		n, _ := binary.ReadUvarint(r)
		for i := uint64(0); i < n; i++ {
			ct += readRuleCompatWithRules(r, m, last, rustRules)
		}
		r.ReadByte()
	}
	return ct
}

func readDefaultRuleCompat(r *bufio.Reader, m *domain.MixMatcher[struct{}], last *string) int {
	return readDefaultRuleCompatWithRules(r, m, last, nil)
}

func readDefaultRuleCompatWithRules(r *bufio.Reader, m *domain.MixMatcher[struct{}], last *string, rustRules *[]string) int {
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
				rule := "full:" + d
				*last = rule
				if m.Add(rule, struct{}{}) == nil {
					count++
					if rustRules != nil {
						*rustRules = append(*rustRules, rule)
					}
				}
			}
			for _, d := range suffix {
				rule := "domain:" + d
				*last = rule
				if m.Add(rule, struct{}{}) == nil {
					count++
					if rustRules != nil {
						*rustRules = append(*rustRules, rule)
					}
				}
			}
		case ruleItemDomainKeyword:
			sl, _ := varbin.ReadValue[[]string](r, binary.BigEndian)
			for _, d := range sl {
				rule := "keyword:" + d
				*last = rule
				if m.Add(rule, struct{}{}) == nil {
					count++
					if rustRules != nil {
						*rustRules = append(*rustRules, rule)
					}
				}
			}
		case ruleItemDomainRegex:
			sl, _ := varbin.ReadValue[[]string](r, binary.BigEndian)
			for _, d := range sl {
				rule := "regexp:" + d
				*last = rule
				if m.Add(rule, struct{}{}) == nil {
					count++
					if rustRules != nil {
						*rustRules = append(*rustRules, rule)
					}
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
