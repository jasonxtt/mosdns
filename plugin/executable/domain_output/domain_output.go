package domain_output

import (
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

	"github.com/go-chi/chi/v5"
	"github.com/IrineSistiana/mosdns/v5/coremain"
	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
	"github.com/IrineSistiana/mosdns/v5/plugin/executable/sequence"
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
}

type domainOutput struct {
	fileStat       string
	fileRule       string
	genRule        string
	pattern        string
	appendedString string
	maxEntries     int
	dumpInterval   time.Duration

	stats        map[string]int
	mu           sync.Mutex
	totalCount   int
	entryCounter int

	writeSignalChan chan struct{}
	stopChan        chan struct{}
	workerDoneChan  chan struct{}

	domainSetURL string

	// [新增修复] 确保 Close 只执行一次
	closeOnce sync.Once
}

type WriteMode int

const (
	WriteModePeriodic WriteMode = iota
	WriteModeFlush
	WriteModeSave
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
		stats:           make(map[string]int),
		writeSignalChan: make(chan struct{}, 1),
		stopChan:        make(chan struct{}),
		workerDoneChan:  make(chan struct{}),
		domainSetURL:    cfg.DomainSetURL,
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
		stats:           make(map[string]int),
		writeSignalChan: make(chan struct{}, 1),
		stopChan:        make(chan struct{}),
		workerDoneChan:  make(chan struct{}),
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
	for _, question := range qCtx.Q().Question {
		domain := strings.TrimSuffix(question.Name, ".")
		d.stats[domain]++
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

	var statsToDump map[string]int

	switch mode {
	case WriteModePeriodic:
		statsToDump = make(map[string]int, len(d.stats))
		for k, v := range d.stats {
			statsToDump[k] = v
		}
		if len(statsToDump) == 0 {
			d.mu.Unlock()
			return
		}
		d.entryCounter = 0
	case WriteModeFlush:
		statsToDump = make(map[string]int)
		d.stats = make(map[string]int)
		d.totalCount = 0
		d.entryCounter = 0
	case WriteModeSave:
		statsToDump = make(map[string]int, len(d.stats))
		for k, v := range d.stats {
			statsToDump[k] = v
		}
		d.entryCounter = 0
	}

	d.mu.Unlock()

	d.doWriteFiles(statsToDump)

	if len(statsToDump) > 0 || mode == WriteModeFlush || mode == WriteModeSave {
		d.pushToDomainSet(statsToDump)
	}
}

func (d *domainOutput) doWriteFiles(statsData map[string]int) {
	writeFile := func(filePath string, writeContent func(io.Writer) error) {
		if filePath == "" {
			return
		}
		file, err := os.Create(filePath)
		if err != nil {
			fmt.Printf("[domain_output] failed to create file %s: %v\n", filePath, err)
			return
		}
		defer file.Close()

		if err := writeContent(file); err != nil {
			fmt.Printf("[domain_output] failed to write to file %s: %v\n", filePath, err)
		}
	}

	// 写入 stat 文件
	writeFile(d.fileStat, func(w io.Writer) error {
		for domain, count := range statsData {
			if _, err := w.Write([]byte(fmt.Sprintf("%010d %s\n", count, domain))); err != nil {
				return err
			}
		}
		return nil
	})

	// 写入 rule 文件
	writeFile(d.fileRule, func(w io.Writer) error {
		for domain := range statsData {
			if _, err := w.Write([]byte(fmt.Sprintf("full:%s\n", domain))); err != nil {
				return err
			}
		}
		return nil
	})

	// 写入 genRule 文件
	writeFile(d.genRule, func(w io.Writer) error {
		if d.pattern == "" {
			return nil
		}
		if d.appendedString != "" {
			if _, err := w.Write([]byte(d.appendedString + "\n")); err != nil {
				return err
			}
		}
		for domain := range statsData {
			line := strings.ReplaceAll(d.pattern, "DOMAIN", domain)
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

	var domain string
	var count int
	for {
		_, err := fmt.Fscanf(file, "%d %s\n", &count, &domain)
		if err != nil {
			break
		}
		d.stats[domain] = count
		d.totalCount += count
	}
	fmt.Printf("[domain_output] loaded %d entries from %s\n", len(d.stats), d.fileStat)
}

func (d *domainOutput) pushToDomainSet(statsData map[string]int) {
	if d.domainSetURL == "" {
		return
	}

	vals := make([]string, 0, len(statsData))
	for domain := range statsData {
		vals = append(vals, fmt.Sprintf("full:%s", domain))
	}

	payload := struct{ Values []string `json:"values"` }{Values: vals}
	body, err := json.Marshal(payload)
	if err != nil {
		fmt.Printf("[domain_output] marshal payload error: %v\n", err)
		return
	}

	go func() {
		// [优化] 添加超时 context，避免阻塞
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

		d.performWrite(WriteModeSave)

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
		statsCopy := make(map[string]int, len(d.stats))
		for domain, count := range d.stats {
			statsCopy[domain] = count
		}
		d.mu.Unlock()

		type domainStat struct {
			Domain string
			Count  int
		}
		statsSlice := make([]domainStat, 0, len(statsCopy))
		for domain, count := range statsCopy {
			statsSlice = append(statsSlice, domainStat{Domain: domain, Count: count})
		}
		sort.Slice(statsSlice, func(i, j int) bool {
			return statsSlice[i].Count > statsSlice[j].Count
		})

		for _, stat := range statsSlice {
			if _, err := fmt.Fprintf(w, "%010d %s\n", stat.Count, stat.Domain); err != nil {
				fmt.Printf("[domain_output] failed to write to http response: %v\n", err)
				return
			}
		}
	})

	r.Get("/restartall", func(w http.ResponseWriter, req *http.Request) {
		d.performWrite(WriteModeSave)
		w.WriteHeader(http.StatusOK)
		w.Write([]byte("mosdns restarted"))
		go restartSelf()
	})

	return r
}
