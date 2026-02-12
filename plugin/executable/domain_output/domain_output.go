package domain_output

import (
	"bufio"
	"bytes"
	"context"
	"container/heap"
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
	"sync/atomic"
	"syscall"
	"time"

	"github.com/IrineSistiana/mosdns/v5/coremain"
	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
	"github.com/IrineSistiana/mosdns/v5/plugin/executable/sequence"
	"github.com/go-chi/chi/v5"
)

const PluginType = "domain_output"
const RecordBufferLimit = 10240

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

type statEntry struct {
	Count    int
	LastDate string
}

// logItem carries raw data from Exec to background worker
type logItem struct {
	name string
	ad   bool
	cd   bool
	do   bool
}

type domainOutput struct {
	fileStat       string
	fileRule       string
	genRule        string
	pattern        string
	appendedString string
	maxEntries     int
	dumpInterval   time.Duration

	stats        map[string]*statEntry
	mu           sync.Mutex
	
	// Atomic counters for performance
	totalCount   int64
	entryCounter int64

	currentDate atomic.Value // stores string

	recordChan      chan *logItem
	writeSignalChan chan struct{}
	stopChan        chan struct{}
	workerDoneChan  chan struct{}

	domainSetURL string
	enableFlags  bool

	closeOnce sync.Once
}

type WriteMode int

const (
	WriteModePeriodic WriteMode = iota
	WriteModeFlush
	WriteModeSave
	WriteModeShutdown
)

type outputRankItem struct {
	Domain string
	Count  int
	Date   string
}

type outputRankHeap []outputRankItem

func (h outputRankHeap) Len() int           { return len(h) }
func (h outputRankHeap) Less(i, j int) bool { return h[i].Count < h[j].Count }
func (h outputRankHeap) Swap(i, j int)      { h[i], h[j] = h[j], h[i] }
func (h *outputRankHeap) Push(x any)        { *h = append(*h, x.(outputRankItem)) }
func (h *outputRankHeap) Pop() any {
	old := *h
	n := len(old)
	x := old[n-1]
	*h = old[0 : n-1]
	return x
}

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
		recordChan:      make(chan *logItem, RecordBufferLimit),
		writeSignalChan: make(chan struct{}, 1),
		stopChan:        make(chan struct{}),
		workerDoneChan:  make(chan struct{}),
		domainSetURL:    cfg.DomainSetURL,
		enableFlags:     cfg.EnableFlags,
	}
	d.currentDate.Store(time.Now().Format("2006-01-02"))
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
		recordChan:      make(chan *logItem, RecordBufferLimit),
		writeSignalChan: make(chan struct{}, 1),
		stopChan:        make(chan struct{}),
		workerDoneChan:  make(chan struct{}),
		currentDate:     atomic.Value{},
	}
	d.currentDate.Store(time.Now().Format("2006-01-02"))
	if len(params) == 7 {
		d.domainSetURL = params[6]
	}
	d.loadFromFile()

	go d.startWorker()

	return d, nil
}

func (d *domainOutput) Exec(ctx context.Context, qCtx *query_context.Context) error {
	q := qCtx.Q()
	if q == nil || len(q.Question) == 0 {
		return nil
	}

	for _, question := range q.Question {
		item := &logItem{
			name: question.Name,
		}

		if d.enableFlags {
			item.ad = q.AuthenticatedData
			item.cd = q.CheckingDisabled
			if opt := q.IsEdns0(); opt != nil {
				item.do = opt.Do()
			}
		}

		// Non-blocking send. If channel is full, item is dropped to protect latency.
		select {
		case d.recordChan <- item:
		default:
		}
	}

	return nil
}

func (d *domainOutput) startWorker() {
	ticker := time.NewTicker(d.dumpInterval)
	defer ticker.Stop()
	defer close(d.workerDoneChan)

	for {
		select {
		case item := <-d.recordChan:
			d.processRecord(item)

		case <-ticker.C:
			d.performWrite(WriteModePeriodic)

		case <-d.writeSignalChan:
			d.performWrite(WriteModePeriodic)

		case <-d.stopChan:
			// Drain remaining items before stopping
			for {
				select {
				case item := <-d.recordChan:
					d.processRecord(item)
				default:
					return
				}
			}
		}
	}
}

func (d *domainOutput) processRecord(item *logItem) {
	// Move string operations to background worker
	rawDomain := strings.TrimSuffix(item.name, ".")
	storageKey := rawDomain

	if d.enableFlags {
		var flags []string
		if item.ad {
			flags = append(flags, "AD")
		}
		if item.cd {
			flags = append(flags, "CD")
		}
		if item.do {
			flags = append(flags, "DO")
		}
		if len(flags) > 0 {
			storageKey = rawDomain + "|" + strings.Join(flags, "|")
		}
	}

	d.mu.Lock()
	currDate := d.currentDate.Load().(string)
	entry, exists := d.stats[storageKey]
	if !exists {
		entry = &statEntry{
			Count:    0,
			LastDate: currDate,
		}
		d.stats[storageKey] = entry
	}
	entry.Count++
	if entry.LastDate != currDate {
		entry.LastDate = currDate
	}
	d.mu.Unlock()

	atomic.AddInt64(&d.totalCount, 1)
	newCount := atomic.AddInt64(&d.entryCounter, 1)

	if d.maxEntries > 0 && newCount >= int64(d.maxEntries) {
		select {
		case d.writeSignalChan <- struct{}{}:
		default:
		}
	}
}

func (d *domainOutput) performWrite(mode WriteMode) {
	// Update current date cache
	d.currentDate.Store(time.Now().Format("2006-01-02"))

	var statsToDump map[string]*statEntry

	d.mu.Lock()
	switch mode {
	case WriteModePeriodic:
		statsToDump = make(map[string]*statEntry, len(d.stats))
		for k, v := range d.stats {
			statsToDump[k] = &statEntry{Count: v.Count, LastDate: v.LastDate}
		}
		if len(statsToDump) == 0 {
			d.mu.Unlock()
			return
		}
		atomic.StoreInt64(&d.entryCounter, 0)
	case WriteModeFlush:
		statsToDump = make(map[string]*statEntry)
		d.stats = make(map[string]*statEntry)
		atomic.StoreInt64(&d.totalCount, 0)
		atomic.StoreInt64(&d.entryCounter, 0)
	case WriteModeSave, WriteModeShutdown:
		statsToDump = make(map[string]*statEntry, len(d.stats))
		for k, v := range d.stats {
			statsToDump[k] = &statEntry{Count: v.Count, LastDate: v.LastDate}
		}
		atomic.StoreInt64(&d.entryCounter, 0)
	}
	d.mu.Unlock()

	d.doWriteFiles(statsToDump)

	if mode != WriteModeShutdown {
		d.pushToDomainSet(statsToDump)
	}
	statsToDump = nil
	coremain.ManualGC()
}

func (d *domainOutput) doWriteFiles(statsData map[string]*statEntry) {
	writeFile := func(filePath string, writeContent func(io.Writer) error) {
		if filePath == "" {
			return
		}
		file, err := os.OpenFile(filePath, os.O_RDWR|os.O_CREATE|os.O_TRUNC, 0644)
		if err != nil {
			return
		}
		defer file.Close()
		_ = writeContent(file)
	}

	type sortItem struct {
		Key   string
		Entry *statEntry
	}
	sortedItems := make([]sortItem, 0, len(statsData))
	for k, v := range statsData {
		sortedItems = append(sortedItems, sortItem{Key: k, Entry: v})
	}

	sort.Slice(sortedItems, func(i, j int) bool {
		return sortedItems[i].Entry.Count > sortedItems[j].Entry.Count
	})

	writeFile(d.fileStat, func(w io.Writer) error {
		for _, item := range sortedItems {
			line := fmt.Sprintf("%010d %s %s\n", item.Entry.Count, item.Entry.LastDate, item.Key)
			_, _ = w.Write([]byte(line))
		}
		return nil
	})

	writeFile(d.fileRule, func(w io.Writer) error {
		seen := make(map[string]bool)
		for _, item := range sortedItems {
			domainOnly := strings.Split(item.Key, "|")[0]
			if seen[domainOnly] {
				continue
			}
			seen[domainOnly] = true
			_, _ = w.Write([]byte(fmt.Sprintf("full:%s\n", domainOnly)))
		}
		return nil
	})

	writeFile(d.genRule, func(w io.Writer) error {
		if d.pattern == "" {
			return nil
		}
		if d.appendedString != "" {
			_, _ = w.Write([]byte(d.appendedString + "\n"))
		}
		seen := make(map[string]bool)
		for _, item := range sortedItems {
			domainOnly := strings.Split(item.Key, "|")[0]
			if seen[domainOnly] {
				continue
			}
			seen[domainOnly] = true
			line := strings.ReplaceAll(d.pattern, "DOMAIN", domainOnly)
			_, _ = w.Write([]byte(line + "\n"))
		}
		return nil
	})
}

func (d *domainOutput) loadFromFile() {
	file, err := os.Open(d.fileStat)
	if err != nil {
		return
	}
	defer file.Close()

	d.mu.Lock()
	defer d.mu.Unlock()

	scanner := bufio.NewScanner(file)
	today := time.Now().Format("2006-01-02")

	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		if line == "" {
			continue
		}
		fields := strings.Fields(line)
		
		var count int
		var domain string
		var date string

		if len(fields) == 2 {
			c, _ := strconv.Atoi(fields[0])
			count = c
			domain = fields[1]
			date = today
		} else if len(fields) >= 3 {
			c, _ := strconv.Atoi(fields[0])
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
		atomic.AddInt64(&d.totalCount, int64(count))
	}
	coremain.ManualGC()
}

func (d *domainOutput) pushToDomainSet(statsData map[string]*statEntry) {
	if d.domainSetURL == "" {
		return
	}

	seen := make(map[string]bool)
	vals := make([]string, 0, len(statsData))

	for key := range statsData {
		domainOnly := strings.Split(key, "|")[0]
		if seen[domainOnly] {
			continue
		}
		seen[domainOnly] = true
		vals = append(vals, fmt.Sprintf("full:%s", domainOnly))
	}

	payload := struct{ Values []string `json:"values"` }{Values: vals}
	body, _ := json.Marshal(payload)

	go func() {
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()

		req, err := http.NewRequestWithContext(ctx, "POST", d.domainSetURL, bytes.NewReader(body))
		if err != nil {
			return
		}
		req.Header.Set("Content-Type", "application/json")
		resp, err := http.DefaultClient.Do(req)
		if err != nil {
			return
		}
		_, _ = io.Copy(io.Discard, resp.Body)
		resp.Body.Close()
	}()
}

func (d *domainOutput) Close() error {
	d.closeOnce.Do(func() {
		close(d.stopChan)
		<-d.workerDoneChan
		d.performWrite(WriteModeShutdown)
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
	_ = syscall.Exec(bin, args, env)
}

func (d *domainOutput) Api() *chi.Mux {
	r := chi.NewRouter()

	r.Get("/flush", coremain.WithAsyncGC(func(w http.ResponseWriter, r *http.Request) {
		d.performWrite(WriteModeFlush)
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write([]byte("domain_output flushed"))
	}))

	r.Get("/save", coremain.WithAsyncGC(func(w http.ResponseWriter, r *http.Request) {
		d.performWrite(WriteModeSave)
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write([]byte("domain_output saved"))
	}))

	r.Get("/show", coremain.WithAsyncGC(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/plain; charset=utf-8")
		query := strings.ToLower(r.URL.Query().Get("q"))
		limit, _ := strconv.Atoi(r.URL.Query().Get("limit"))
		offset, _ := strconv.Atoi(r.URL.Query().Get("offset"))

		if limit <= 0 { limit = 100 }
		if offset < 0 { offset = 0 }

		h := &outputRankHeap{}
		heap.Init(h)
		maxHeapSize := offset + limit

		d.mu.Lock()
		totalFiltered := 0 
		for domain, entry := range d.stats {
			if query != "" && !strings.Contains(strings.ToLower(domain), query) {
				continue
			}
			totalFiltered++ 
			item := outputRankItem{Domain: domain, Count: entry.Count, Date: entry.LastDate}
			if h.Len() < maxHeapSize {
				heap.Push(h, item)
			} else if item.Count > (*h)[0].Count {
				heap.Pop(h)
				heap.Push(h, item)
			}
		}
		d.mu.Unlock()

		w.Header().Set("X-Total-Count", strconv.Itoa(totalFiltered))
		w.Header().Set("Access-Control-Expose-Headers", "X-Total-Count")

		resultCount := h.Len()
		sortedResult := make([]outputRankItem, resultCount)
		for i := resultCount - 1; i >= 0; i-- {
			sortedResult[i] = heap.Pop(h).(outputRankItem)
		}

		if offset < resultCount {
			for i := offset; i < resultCount; i++ {
				stat := sortedResult[i]
				_, _ = fmt.Fprintf(w, "%010d %s %s\n", stat.Count, stat.Date, stat.Domain)
			}
		}
	}))

	r.Get("/restartall", func(w http.ResponseWriter, req *http.Request) {
		_ = d.Close()
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write([]byte("mosdns restarting"))
		go restartSelf()
	})

	return r
}
