package domain_output

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"sort"
	"strconv"
	"strings"
	"sync"
	"syscall"
	"time"

	"github.com/IrineSistiana/mosdns/v5/coremain"
	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
	"github.com/IrineSistiana/mosdns/v5/plugin/executable/sequence"
	"github.com/go-chi/chi/v5"
)

const PluginType = "domain_output"

func init() {
	coremain.RegNewPluginFunc(PluginType, Init, func() any { return new(Args) })
	sequence.MustRegExecQuickSetup(PluginType, QuickSetup)
}

type Args struct {
	FileStat       string `yaml:"file_stat"`
	FileRule       string `yaml:"file_rule"`
	GenRule        string `yaml:"gen_rule"`
	Pattern        string `yaml:"pattern"`
	AppendedString string `yaml:"appended_string"`
	MaxEntries     int    `yaml:"max_entries"`
	DumpInterval   int    `yaml:"dump_interval"`
	DomainSetURL   string `yaml:"domain_set_url"`
	EnableFlags    bool   `yaml:"enable_flags"`
}

// statEntry 存储域名统计信息：次数和最后访问日期
type statEntry struct {
	Count    int
	LastDate string
}

type domainOutput struct {
	fileStat       string
	fileRule       string
	genRule        string
	pattern        string
	appendedString string
	maxEntries     int
	dumpInterval   time.Duration

	// 修改 stats 类型以存储更多信息
	stats        map[string]*statEntry
	mu           sync.Mutex
	totalCount   int
	entryCounter int

	// 缓存当前日期字符串，避免在高频 Exec 中频繁调用 time.Now().Format
	currentDate string

	writeSignalChan chan struct{}
	stopChan        chan struct{}
	workerDoneChan  chan struct{}

	domainSetURL string
	enableFlags    bool

	// [新增修复] 确保 Close 只执行一次
	closeOnce sync.Once
}

type WriteMode int

const (
	WriteModePeriodic WriteMode = iota
	WriteModeFlush
	WriteModeSave
        WriteModeShutdown
)

func Init(bp *coremain.BP, args any) (any, error) {
	cfg := args.(*Args)
	if cfg.DumpInterval <= 0 {
		cfg.DumpInterval = 60
	}
	d := &domainOutput{
		fileStat:        cfg.FileStat,
		fileRule:        cfg.FileRule,
		genRule:         cfg.GenRule,
		pattern:         cfg.Pattern,
		appendedString:  cfg.AppendedString,
		maxEntries:      cfg.MaxEntries,
		dumpInterval:    time.Duration(cfg.DumpInterval) * time.Second,
		stats:           make(map[string]*statEntry),
		writeSignalChan: make(chan struct{}, 1),
		stopChan:        make(chan struct{}),
		workerDoneChan:  make(chan struct{}),
		domainSetURL:    cfg.DomainSetURL,
                enableFlags:     cfg.EnableFlags,
		currentDate:     time.Now().Format("2006-01-02"), // 初始化当前日期
	}
	d.loadFromFile()

	go d.startWorker()
	bp.RegAPI(d.Api())

	return d, nil
}

func QuickSetup(_ sequence.BQ, s string) (any, error) {
	params := strings.Split(s, ",")
	if len(params) < 6 || len(params) > 7 {
		return nil, errors.New("invalid quick setup arguments: need 6 or 7 fields")
	}
	fileStat := params[0]
	fileRule := params[1]
	genRule := params[2]
	pattern := params[3]
	maxEntries, err := strconv.Atoi(params[4])
	if err != nil {
		return nil, err
	}
	dumpInterval, err := strconv.Atoi(params[5])
	if err != nil || dumpInterval <= 0 {
		dumpInterval = 60
	}
	d := &domainOutput{
		fileStat:        fileStat,
		fileRule:        fileRule,
		genRule:         genRule,
		pattern:         pattern,
		maxEntries:      maxEntries,
		dumpInterval:    time.Duration(dumpInterval) * time.Second,
		stats:           make(map[string]*statEntry),
		writeSignalChan: make(chan struct{}, 1),
		stopChan:        make(chan struct{}),
		workerDoneChan:  make(chan struct{}),
		currentDate:     time.Now().Format("2006-01-02"),
	}
	if len(params) == 7 {
		d.domainSetURL = params[6]
	}
	d.loadFromFile()

	go d.startWorker()

	return d, nil
}

func (d *domainOutput) Exec(ctx context.Context, qCtx *query_context.Context) error {
	d.mu.Lock()

	// [修复 1] 获取 DNS 请求对象
	q := qCtx.Q()
	
	// [修复 2] 初始化后缀变量，默认为空
	suffix := ""
	
	// --- 新增逻辑：根据 Flag 生成后缀 ---
	if d.enableFlags {
		var flags []string
		if q.AuthenticatedData {
			flags = append(flags, "AD")
		}
		if q.CheckingDisabled {
			flags = append(flags, "CD")
		}
		if opt := q.IsEdns0(); opt != nil && opt.Do() {
			flags = append(flags, "DO")
		}

		if len(flags) > 0 {
			suffix = "|" + strings.Join(flags, "|")
		}
	}
	// ----------------------------------

	for _, question := range q.Question {
		rawDomain := strings.TrimSuffix(question.Name, ".")
		// Key 变为: 域名 + 后缀 (如果 enableFlags 为 false，suffix 为空，Key 就是域名本身)
		storageKey := rawDomain + suffix
		
		entry, exists := d.stats[storageKey]
		if !exists {
			entry = &statEntry{
				Count:    0,
				LastDate: d.currentDate,
			}
			d.stats[storageKey] = entry
		}
		
		entry.Count++
		if entry.LastDate != d.currentDate {
			entry.LastDate = d.currentDate
		}

		d.totalCount++
		d.entryCounter++
	}
	if d.entryCounter >= d.maxEntries {
		select {
		case d.writeSignalChan <- struct{}{}:
		default:
		}
	}
	d.mu.Unlock()

	return nil
}

func (d *domainOutput) startWorker() {
	ticker := time.NewTicker(d.dumpInterval)
	defer ticker.Stop()
	defer close(d.workerDoneChan)

	for {
		select {
		case <-ticker.C:
			d.performWrite(WriteModePeriodic)
		case <-d.writeSignalChan:
			d.performWrite(WriteModePeriodic)
		case <-d.stopChan:
			fmt.Println("[domain_output] worker received stop signal, stopping.")
			return
		}
	}
}

func (d *domainOutput) performWrite(mode WriteMode) {
	d.mu.Lock()

	// 在每次写入操作前更新当前日期缓存，这样 Exec 中就不需要频繁调用 time.Now()
	// 误差最多为一个 dumpInterval，对于“最近一次访问日期”是完全可接受的
	d.currentDate = time.Now().Format("2006-01-02")

	var statsToDump map[string]*statEntry

	switch mode {
	case WriteModePeriodic:
		statsToDump = make(map[string]*statEntry, len(d.stats))
		for k, v := range d.stats {
			// 复制一份数据快照
			statsToDump[k] = &statEntry{Count: v.Count, LastDate: v.LastDate}
		}
		if len(statsToDump) == 0 {
			d.mu.Unlock()
			return
		}
		d.entryCounter = 0
	case WriteModeFlush:
		statsToDump = make(map[string]*statEntry)
		d.stats = make(map[string]*statEntry)
		d.totalCount = 0
		d.entryCounter = 0
	case WriteModeSave, WriteModeShutdown:
		statsToDump = make(map[string]*statEntry, len(d.stats))
		for k, v := range d.stats {
			statsToDump[k] = &statEntry{Count: v.Count, LastDate: v.LastDate}
		}
		d.entryCounter = 0
	}

	d.mu.Unlock()

	d.doWriteFiles(statsToDump)

	if mode != WriteModeShutdown {
	    d.pushToDomainSet(statsToDump)
	}
}

func (d *domainOutput) doWriteFiles(statsData map[string]*statEntry) {
	writeFile := func(filePath string, writeContent func(io.Writer) error) {
		if filePath == "" {
			return
		}
		// 使用 O_TRUNC 确保文件被重写
		file, err := os.OpenFile(filePath, os.O_RDWR|os.O_CREATE|os.O_TRUNC, 0644)
		if err != nil {
			fmt.Printf("[domain_output] failed to create file %s: %v\n", filePath, err)
			return
		}
		defer file.Close()

		if err := writeContent(file); err != nil {
			fmt.Printf("[domain_output] failed to write to file %s: %v\n", filePath, err)
		}
	}

	// 准备排序数据
	type sortItem struct {
		Key   string 
		Entry *statEntry
	}
	sortedItems := make([]sortItem, 0, len(statsData))
	for k, v := range statsData {
		sortedItems = append(sortedItems, sortItem{Key: k, Entry: v})
	}

	// 按总访问次数从大到小排序
	sort.Slice(sortedItems, func(i, j int) bool {
		return sortedItems[i].Entry.Count > sortedItems[j].Entry.Count
	})

	// 1. 写入 stat 文件 (包含 Flags 信息，供 requery 精准还原)
	// 这里的 item.Key 可能是 "google.com" 也可能是 "google.com|AD"
	writeFile(d.fileStat, func(w io.Writer) error {
		for _, item := range sortedItems {
			line := fmt.Sprintf("%010d %s %s\n", item.Entry.Count, item.Entry.LastDate, item.Key)
			if _, err := w.Write([]byte(line)); err != nil {
				return err
			}
		}
		return nil
	})

	// 2. 写入 rule 文件 (必须剔除 Flags 并去重，保持 full:example.com 纯净)
	writeFile(d.fileRule, func(w io.Writer) error {
		seen := make(map[string]bool) // 用于去重

		for _, item := range sortedItems {
			// 分割 Key，只取域名部分。如果是 "google.com"，parts[0] 就是它自己。
			domainOnly := strings.Split(item.Key, "|")[0]

			if seen[domainOnly] {
				continue
			}
			seen[domainOnly] = true

			if _, err := w.Write([]byte(fmt.Sprintf("full:%s\n", domainOnly))); err != nil {
				return err
			}
		}
		return nil
	})

	// 3. 写入 genRule 文件 (同样剔除 Flags)
	writeFile(d.genRule, func(w io.Writer) error {
		if d.pattern == "" {
			return nil
		}
		if d.appendedString != "" {
			if _, err := w.Write([]byte(d.appendedString + "\n")); err != nil {
				return err
			}
		}
		
		seen := make(map[string]bool)

		for _, item := range sortedItems {
			domainOnly := strings.Split(item.Key, "|")[0]
			if seen[domainOnly] {
				continue
			}
			seen[domainOnly] = true

			line := strings.ReplaceAll(d.pattern, "DOMAIN", domainOnly)
			if _, err := w.Write([]byte(line + "\n")); err != nil {
				return err
			}
		}
		return nil
	})
}

func (d *domainOutput) loadFromFile() {
	file, err := os.Open(d.fileStat)
	if err != nil {
		if !os.IsNotExist(err) {
			fmt.Printf("[domain_output] failed to open stat file %s: %v\n", d.fileStat, err)
		}
		return
	}
	defer file.Close()

	d.mu.Lock()
	defer d.mu.Unlock()

	scanner := bufio.NewScanner(file)
	today := time.Now().Format("2006-01-02")
	loadedCount := 0

	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		if line == "" {
			continue
		}
		fields := strings.Fields(line)
		
		var count int
		var domain string
		var date string

		// 兼容逻辑
		if len(fields) == 2 {
			// 旧格式: count domain
			c, err := strconv.Atoi(fields[0])
			if err != nil {
				continue
			}
			count = c
			domain = fields[1]
			date = today // 旧格式默认使用今天
		} else if len(fields) >= 3 {
			// 新格式: count date domain
			c, err := strconv.Atoi(fields[0])
			if err != nil {
				continue
			}
			count = c
			date = fields[1]
			domain = fields[2]
		} else {
			continue
		}

		d.stats[domain] = &statEntry{
			Count:    count,
			LastDate: date,
		}
		d.totalCount += count
		loadedCount++
	}

	fmt.Printf("[domain_output] loaded %d entries from %s\n", loadedCount, d.fileStat)
}

func (d *domainOutput) pushToDomainSet(statsData map[string]*statEntry) {
	if d.domainSetURL == "" {
		return
	}

	// [修改 1] 初始化去重 Map
	// 无论 enable_flags 是否开启，去重都是安全的。
	// 特别是当 enable_flags=true 时，必须把 "a.com|AD" 和 "a.com|DO" 合并为同一个 "a.com"
	seen := make(map[string]bool)
	vals := make([]string, 0, len(statsData))

	for key := range statsData {
		// [修改 2] 强制剥离后缀
		// strings.Split(key, "|")[0] 可以处理两种情况：
		// A. key="google.com" (纯净) -> 结果 "google.com"
		// B. key="google.com|AD" (带标) -> 结果 "google.com"
		// 这保证了发给 domain_set 的永远是用于匹配的纯域名。
		domainOnly := strings.Split(key, "|")[0]

		if seen[domainOnly] {
			continue
		}
		seen[domainOnly] = true

		vals = append(vals, fmt.Sprintf("full:%s", domainOnly))
	}

	// [修改 3] 如果过滤去重后没有数据，直接返回，不发送请求
	if len(vals) == 0 {
		return
	}

	payload := struct{ Values []string `json:"values"` }{Values: vals}
	body, err := json.Marshal(payload)
	if err != nil {
		fmt.Printf("[domain_output] marshal payload error: %v\n", err)
		return
	}

	go func() {
		// [保持优化] 使用带超时的 Context，防止 API 卡死
		ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
		defer cancel()

		req, err := http.NewRequestWithContext(ctx, "POST", d.domainSetURL, bytes.NewReader(body))
		if err != nil {
			fmt.Printf("[domain_output] create POST request error: %v\n", err)
			return
		}
		req.Header.Set("Content-Type", "application/json")
		resp, err := http.DefaultClient.Do(req)
		if err != nil {
			// 在关闭时连接被拒绝是正常的，因为服务器可能正在重启
			fmt.Printf("[domain_output] POST to domain_set error: %v\n", err)
			return
		}
		io.Copy(io.Discard, resp.Body)
		resp.Body.Close()
		fmt.Printf("[domain_output] pushed %d rules to domain_set, status=%s\n", len(vals), resp.Status)
	}()
}

// Close closes the plugin.
// [修复] 使用 sync.Once 确保 Close 只执行一次，防止 channel 重复关闭 panic
func (d *domainOutput) Close() error {
	d.closeOnce.Do(func() {
		fmt.Println("[domain_output] initiating shutdown...")
		close(d.stopChan)
		<-d.workerDoneChan

		d.performWrite(WriteModeShutdown)

		fmt.Println("[domain_output] shutdown complete.")
	})
	return nil
}

func restartSelf() {
	time.Sleep(100 * time.Millisecond)

	bin, err := os.Executable()
	if err != nil {
		os.Exit(0)
	}
	args := os.Args
	env := os.Environ()
	syscall.Exec(bin, args, env)
}

func (d *domainOutput) Api() *chi.Mux {
	r := chi.NewRouter()

	r.Get("/flush", func(w http.ResponseWriter, req *http.Request) {
		d.performWrite(WriteModeFlush)
		w.WriteHeader(http.StatusOK)
		w.Write([]byte("domain_output flushed and files rewritten."))
	})

	r.Get("/save", func(w http.ResponseWriter, req *http.Request) {
		d.performWrite(WriteModeSave)
		w.WriteHeader(http.StatusOK)
		w.Write([]byte("domain_output files saved."))
	})

	r.Get("/show", func(w http.ResponseWriter, req *http.Request) {
		w.Header().Set("Content-Type", "text-plain; charset=utf-8")

		d.mu.Lock()
		// 复制数据用于展示
		type domainStat struct {
			Domain string
			Count  int
			Date   string
		}
		statsSlice := make([]domainStat, 0, len(d.stats))
		for domain, entry := range d.stats {
			statsSlice = append(statsSlice, domainStat{
				Domain: domain,
				Count:  entry.Count,
				Date:   entry.LastDate,
			})
		}
		d.mu.Unlock()

		// 排序
		sort.Slice(statsSlice, func(i, j int) bool {
			return statsSlice[i].Count > statsSlice[j].Count
		})

		for _, stat := range statsSlice {
			if _, err := fmt.Fprintf(w, "%010d %s %s\n", stat.Count, stat.Date, stat.Domain); err != nil {
				fmt.Printf("[domain_output] failed to write to http response: %v\n", err)
				return
			}
		}
	})

	r.Get("/restartall", func(w http.ResponseWriter, req *http.Request) {
		d.performWrite(WriteModeShutdown)
		w.WriteHeader(http.StatusOK)
		w.Write([]byte("mosdns restarted"))
		go restartSelf()
	})

	return r
}
