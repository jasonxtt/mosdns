package cache

import (
	"context"
	"encoding/binary"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/IrineSistiana/mosdns/v5/coremain"
	"github.com/IrineSistiana/mosdns/v5/pkg/cache"
	"github.com/IrineSistiana/mosdns/v5/pkg/pool"
	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
	"github.com/IrineSistiana/mosdns/v5/pkg/utils"
	"github.com/IrineSistiana/mosdns/v5/plugin/executable/sequence"
	"github.com/go-chi/chi/v5"
	"github.com/klauspost/compress/gzip"
	"github.com/miekg/dns"
	"github.com/prometheus/client_golang/prometheus"
	"go.uber.org/zap"
	"golang.org/x/sync/singleflight"
	"google.golang.org/protobuf/proto"
	"gopkg.in/yaml.v3"
)

const (
	PluginType = "cache"
	// shardCount 定义分片数量。必须是 2 的幂，这里设定为 256。
	// 这意味着会有 256 个独立的锁和 Map，极大降低高并发下的锁竞争。
	shardCount = 256
)

func init() {
	coremain.RegNewPluginFunc(PluginType, Init, func() any { return new(Args) })
	sequence.MustRegExecQuickSetup(PluginType, quickSetupCache)
}

const (
	defaultLazyUpdateTimeout = time.Second * 5
	expiredMsgTtl            = 5

	minimumChangesToDump   = 1024
	dumpHeader             = "mosdns_cache_v2"
	dumpBlockSize          = 128
	dumpMaximumBlockLength = 1 << 20 // 1M block. 8kb pre entry. Should be enough.
)

const (
	adBit = 1 << iota
	cdBit
	doBit
)

var _ sequence.RecursiveExecutable = (*Cache)(nil)

type Args struct {
	Size         int      `yaml:"size"`
	LazyCacheTTL int      `yaml:"lazy_cache_ttl"`
	EnableECS    bool     `yaml:"enable_ecs"`
	ExcludeIPs   []string `yaml:"exclude_ip"`
	DumpFile     string   `yaml:"dump_file"`
	DumpInterval int      `yaml:"dump_interval"`
}

type argsRaw struct {
	Size         int         `yaml:"size"`
	LazyCacheTTL int         `yaml:"lazy_cache_ttl"`
	EnableECS    bool        `yaml:"enable_ecs"`
	ExcludeIP    interface{} `yaml:"exclude_ip"`
	DumpFile     string      `yaml:"dump_file"`
	DumpInterval int         `yaml:"dump_interval"`
}

// UnmarshalYAML supports both scalar (space-separated) and sequence forms for exclude_ip.
func (a *Args) UnmarshalYAML(node *yaml.Node) error {
	var raw argsRaw
	if err := node.Decode(&raw); err != nil {
		return err
	}
	a.Size = raw.Size
	a.LazyCacheTTL = raw.LazyCacheTTL
	a.DumpFile = raw.DumpFile
	a.DumpInterval = raw.DumpInterval
	a.EnableECS = raw.EnableECS

	switch v := raw.ExcludeIP.(type) {
	case string:
		a.ExcludeIPs = strings.Fields(v)
	case []interface{}:
		for _, x := range v {
			if s, ok := x.(string); ok {
				a.ExcludeIPs = append(a.ExcludeIPs, s)
			} else {
				return fmt.Errorf("exclude_ip list contains non-string: %#v", x)
			}
		}
	case nil:
		// nothing
	default:
		return fmt.Errorf("exclude_ip must be string or list, got %T", v)
	}
	return nil
}

func (a *Args) init() {
	utils.SetDefaultUnsignNum(&a.Size, 1024)
	utils.SetDefaultUnsignNum(&a.DumpInterval, 600)
}

type Cache struct {
	args   *Args
	logger *zap.Logger

	// [修改点] 这里原来是 backend *cache.Cache
	// 现在改为数组，存储 256 个独立的分片 cache
	shards [shardCount]*cache.Cache[key, *item]

	lazyUpdateSF singleflight.Group
	closeOnce    sync.Once
	closeNotify  chan struct{}
	updatedKey   atomic.Uint64

	// [修改点] 新增 dumpMu 锁，保护文件写入操作
	dumpMu sync.Mutex

	queryTotal   prometheus.Counter
	hitTotal     prometheus.Counter
	lazyHitTotal prometheus.Counter
	size         prometheus.GaugeFunc

	excludeNets []*net.IPNet // parsed exclude_ip CIDRs
}

type Opts struct {
	Logger     *zap.Logger
	MetricsTag string
}

func Init(bp *coremain.BP, args any) (any, error) {
	c := NewCache(args.(*Args), Opts{
		Logger:     bp.L(),
		MetricsTag: bp.Tag(),
	})

	if err := c.RegMetricsTo(prometheus.WrapRegistererWithPrefix(PluginType+"_", bp.M().GetMetricsReg())); err != nil {
		return nil, fmt.Errorf("failed to register metrics, %w", err)
	}
	bp.RegAPI(c.Api())
	return c, nil
}

func quickSetupCache(bq sequence.BQ, s string) (any, error) {
	size := 0
	if len(s) > 0 {
		i, err := strconv.Atoi(s)
		if err != nil {
			return nil, fmt.Errorf("invalid size, %w", err)
		}
		size = i
	}
	return NewCache(&Args{Size: size}, Opts{Logger: bq.L()}), nil
}

func NewCache(args *Args, opts Opts) *Cache {
	args.init()

	logger := opts.Logger
	if logger == nil {
		logger = zap.NewNop()
	}

	// parse exclude_ip CIDRs
	var excludeNets []*net.IPNet
	for _, cidr := range args.ExcludeIPs {
		_, ipnet, err := net.ParseCIDR(cidr)
		if err != nil {
			logger.Warn("invalid exclude_ip, skip", zap.String("cidr", cidr), zap.Error(err))
			continue
		}

		logger.Debug("parsed exclude_ip network", zap.String("input", cidr), zap.String("network", ipnet.String()))
		excludeNets = append(excludeNets, ipnet)
	}

	// [修改点] 初始化 Cache 结构体
	p := &Cache{
		args:        args,
		logger:      logger,
		closeNotify: make(chan struct{}),
		excludeNets: excludeNets,
	}

	// [修改点] 初始化所有分片
	// 将总容量 Size 平均分配给每个分片，防止总容量超标
	shardSize := args.Size / shardCount
	if shardSize < 4 {
		shardSize = 4 // 最小保护
	}
	for i := 0; i < shardCount; i++ {
		p.shards[i] = cache.New[key, *item](cache.Opts{Size: shardSize})
	}

	lb := map[string]string{"tag": opts.MetricsTag}
	p.queryTotal = prometheus.NewCounter(prometheus.CounterOpts{
		Name:        "query_total",
		Help:        "The total number of processed queries",
		ConstLabels: lb,
	})
	p.hitTotal = prometheus.NewCounter(prometheus.CounterOpts{
		Name:        "hit_total",
		Help:        "The total number of queries that hit the cache",
		ConstLabels: lb,
	})
	p.lazyHitTotal = prometheus.NewCounter(prometheus.CounterOpts{
		Name:        "lazy_hit_total",
		Help:        "The total number of queries that hit the expired cache",
		ConstLabels: lb,
	})
	p.size = prometheus.NewGaugeFunc(prometheus.GaugeOpts{
		Name:        "size_current",
		Help:        "Current cache size in records",
		ConstLabels: lb,
	}, func() float64 {
		// [修改点] 计算 Size 时需要累加所有分片
		var total int
		for i := 0; i < shardCount; i++ {
			total += p.shards[i].Len()
		}
		return float64(total)
	})

	if err := p.loadDump(); err != nil {
		p.logger.Error("failed to load cache dump", zap.Error(err))
	}
	p.startDumpLoop()

	return p
}

// [新增] getShard 使用 FNV-1a 算法计算 Key 的哈希值，并返回对应的分片
// 这是一个极快且分布均匀的非加密哈希算法。
func (c *Cache) getShard(k string) *cache.Cache[key, *item] {
	var h uint32 = 2166136261
	for i := 0; i < len(k); i++ {
		h ^= uint32(k[i])
		h *= 16777619
	}
	// 使用位运算取模 (因为 shardCount 是 256，即 2^8)
	return c.shards[h&(shardCount-1)]
}

func (c *Cache) containsExcluded(msg *dns.Msg) bool {
	if len(c.excludeNets) == 0 {
		return false
	}
	for _, rr := range msg.Answer {
		var ip net.IP
		switch rr := rr.(type) {
		case *dns.A:
			ip = rr.A
		case *dns.AAAA:
			ip = rr.AAAA
		default:
			continue
		}
		for _, net := range c.excludeNets {
			if net.Contains(ip) {
				c.logger.Debug("skip lazy cache: excluded IP", zap.String("cidr", net.String()), zap.String("ip", ip.String()))
				return true
			}
		}
	}
	return false
}

func (c *Cache) RegMetricsTo(r prometheus.Registerer) error {
	for _, collector := range [...]prometheus.Collector{c.queryTotal, c.hitTotal, c.lazyHitTotal, c.size} {
		if err := r.Register(collector); err != nil {
			return err
		}
	}
	return nil
}

func (c *Cache) Exec(ctx context.Context, qCtx *query_context.Context, next sequence.ChainWalker) error {
	c.queryTotal.Inc()
	q := qCtx.Q()

	msgKey := getMsgKey(q, qCtx, c.args.EnableECS)
	if len(msgKey) == 0 {
		return next.ExecNext(ctx, qCtx)
	}

	// [修改点] 先定位分片 backend
	backend := c.getShard(msgKey)

	cachedResp, lazyHit, domainSet := getRespFromCache(msgKey, backend, c.args.LazyCacheTTL > 0, expiredMsgTtl)
	if lazyHit {
		c.lazyHitTotal.Inc()
		c.doLazyUpdate(msgKey, qCtx, next)
	}
	if cachedResp != nil {
		c.hitTotal.Inc()
		cachedResp.Id = q.Id
		qCtx.SetResponse(cachedResp)
		if domainSet != "" {
			qCtx.StoreValue(query_context.KeyDomainSet, domainSet)
		}
		return nil
	}

	err := next.ExecNext(ctx, qCtx)
	r := qCtx.R()

	if r != nil && !c.containsExcluded(r) {
		// [修改点] 传入定位好的分片
		saveRespToCache(msgKey, qCtx, backend, c.args.LazyCacheTTL)
		c.updatedKey.Add(1)
	}

	return err
}

func (c *Cache) doLazyUpdate(msgKey string, qCtx *query_context.Context, next sequence.ChainWalker) {
	qCtxCopy := qCtx.Copy()
	// [修改点] 预先定位分片，避免在闭包中重复计算
	backend := c.getShard(msgKey)

	lazyUpdateFunc := func() (any, error) {
		defer c.lazyUpdateSF.Forget(msgKey)
		qCtx := qCtxCopy

		c.logger.Debug("start lazy cache update", qCtx.InfoField())
		ctx, cancel := context.WithTimeout(context.Background(), defaultLazyUpdateTimeout)
		defer cancel()

		err := next.ExecNext(ctx, qCtx)
		if err != nil {
			c.logger.Warn("failed to update lazy cache", qCtx.InfoField(), zap.Error(err))
		}

		r := qCtx.R()
		if r != nil && !c.containsExcluded(r) {
			// [修改点] 传入分片
			saveRespToCache(msgKey, qCtx, backend, c.args.LazyCacheTTL)
			c.updatedKey.Add(1)
		}
		c.logger.Debug("lazy cache updated", qCtx.InfoField())
		return nil, nil
	}
	c.lazyUpdateSF.DoChan(msgKey, lazyUpdateFunc)
}

func (c *Cache) Close() error {
	if err := c.dumpCache(); err != nil {
		c.logger.Error("failed to dump cache", zap.Error(err))
	}
	c.closeOnce.Do(func() {
		close(c.closeNotify)
	})
	// [修改点] 关闭所有分片
	for i := 0; i < shardCount; i++ {
		_ = c.shards[i].Close()
	}
	return nil
}

func (c *Cache) loadDump() error {
	if len(c.args.DumpFile) == 0 {
		return nil
	}
	f, err := os.Open(c.args.DumpFile)
	if err != nil {
		if os.IsNotExist(err) {
			c.logger.Info("cache dump file not found, skipping load", zap.String("file", c.args.DumpFile))
			return nil
		}
		return err
	}
	defer f.Close()
	en, err := c.readDump(f)
	if err != nil {
		return err
	}
	c.logger.Info("cache dump loaded", zap.Int("entries", en))
	return nil
}

func (c *Cache) startDumpLoop() {
	if len(c.args.DumpFile) == 0 {
		return
	}
	go func() {
		ticker := time.NewTicker(time.Duration(c.args.DumpInterval) * time.Second)
		defer ticker.Stop()
		for {
			select {
			case <-ticker.C:
				keyUpdated := c.updatedKey.Swap(0)
				if keyUpdated < minimumChangesToDump {
					c.updatedKey.Add(keyUpdated)
					continue
				}
				if err := c.dumpCache(); err != nil {
					c.logger.Error("dump cache", zap.Error(err))
				}
			case <-c.closeNotify:
				return
			}
		}
	}()
}

func (c *Cache) dumpCache() error {
	// [修改点] 加锁，防止自动 Dump 和手动 /save 冲突
	c.dumpMu.Lock()
	defer c.dumpMu.Unlock()

	if len(c.args.DumpFile) == 0 {
		return nil
	}
	f, err := os.Create(c.args.DumpFile)
	if err != nil {
		return err
	}
	defer f.Close()

	en, err := c.writeDump(f)
	if err != nil {
		return fmt.Errorf("failed to write dump, %w", err)
	}
	c.logger.Info("cache dumped", zap.Int("entries", en))
	return nil
}

func (c *Cache) Api() *chi.Mux {
	r := chi.NewRouter()

	r.Get("/flush", func(w http.ResponseWriter, req *http.Request) {
		c.logger.Info("flushing cache via api")
		// [修改点] Flush 所有分片
		for i := 0; i < shardCount; i++ {
			c.shards[i].Flush()
		}

		c.updatedKey.Store(0)

		go func() {
			if err := c.dumpCache(); err != nil {
				c.logger.Error("failed to dump cache after flushing", zap.Error(err))
			}
		}()

		w.WriteHeader(http.StatusOK)
		_, _ = w.Write([]byte("Cache flushed and a background dump has been triggered.\n"))
	})

	r.Get("/dump", func(w http.ResponseWriter, req *http.Request) {
		w.Header().Set("content-type", "application/octet-stream")
		_, err := c.writeDump(w)
		if err != nil {
			http.Error(w, err.Error(), http.StatusInternalServerError)
			return
		}
	})

	// [新增] /save 接口：手动保存缓存到磁盘
	r.Get("/save", func(w http.ResponseWriter, req *http.Request) {
		if len(c.args.DumpFile) == 0 {
			http.Error(w, "dump_file is not configured in config file", http.StatusBadRequest)
			return
		}

		c.logger.Info("saving cache to disk via api")
		// 复用 dumpCache (已加锁)
		err := c.dumpCache()
		if err != nil {
			c.logger.Error("failed to save cache via api", zap.Error(err))
			http.Error(w, fmt.Sprintf("failed to save cache: %v", err), http.StatusInternalServerError)
			return
		}

		w.WriteHeader(http.StatusOK)
		_, _ = w.Write([]byte(fmt.Sprintf("Cache successfully saved to %s\n", c.args.DumpFile)))
	})

	r.Post("/load_dump", func(w http.ResponseWriter, req *http.Request) {
		if _, err := c.readDump(req.Body); err != nil {
			http.Error(w, err.Error(), http.StatusBadRequest)
			return
		}
		w.WriteHeader(http.StatusOK)
	})

	r.Get("/show", func(w http.ResponseWriter, req *http.Request) {
		w.Header().Set("Content-Type", "text/plain; charset=utf-8")
		w.Header().Set("Content-Disposition", `inline; filename="cache.txt"`)

		now := time.Now()
		// [修改点] 遍历所有 256 个分片
		for i := 0; i < shardCount; i++ {
			err := c.shards[i].Range(func(k key, v *item, cacheExpirationTime time.Time) error {
				if cacheExpirationTime.Before(now) {
					return nil
				}

				fmt.Fprintf(w, "----- Cache Entry -----\n")
				fmt.Fprintf(w, "Key:           %s\n", keyToString(k))
				if v.domainSet != "" {
					fmt.Fprintf(w, "DomainSet:     %s\n", v.domainSet)
				}
				fmt.Fprintf(w, "StoredTime:    %s\n", v.storedTime.Format(time.RFC3339))
				fmt.Fprintf(w, "MsgExpire:     %s\n", v.expirationTime.Format(time.RFC3339))
				fmt.Fprintf(w, "CacheExpire:   %s\n", cacheExpirationTime.Format(time.RFC3339))
				fmt.Fprintf(w, "DNS Message:\n%s\n", dnsMsgToString(v.resp))
				return nil
			})
			if err != nil {
				c.logger.Warn("failed to iterate shard during show", zap.Int("shard", i), zap.Error(err))
				// 继续遍历其他分片，不直接中断
			}
		}
	})

	return r
}

func keyToString(k key) string {
	data := []byte(k)
	offset := 0
	var parts []string

	if len(data) < offset+1 {
		return fmt.Sprintf("invalid_key(len<1): %x", data)
	}
	flagsByte := data[offset]
	offset++
	var flags []string
	if flagsByte&adBit != 0 {
		flags = append(flags, "AD")
	}
	if flagsByte&cdBit != 0 {
		flags = append(flags, "CD")
	}
	if flagsByte&doBit != 0 {
		flags = append(flags, "DO")
	}

	if len(data) < offset+2 {
		return fmt.Sprintf("invalid_key(len<3): %x", data)
	}
	qtype := binary.BigEndian.Uint16(data[offset : offset+2])
	offset += 2

	if len(data) < offset+1 {
		return fmt.Sprintf("invalid_key(len<4): %x", data)
	}
	nameLen := int(data[offset])
	offset++
	if len(data) < offset+nameLen {
		return fmt.Sprintf("invalid_key(incomplete_name): %x", data)
	}
	qname := string(data[offset : offset+nameLen])
	parts = append(parts, qname, dns.TypeToString[qtype], "IN")
	offset += nameLen

	if len(flags) > 0 {
		parts = append(parts, fmt.Sprintf("[flags:%s]", strings.Join(flags, ",")))
	}

	if offset < len(data) {
		if len(data) < offset+1 {
			parts = append(parts, "[ecs:invalid_len_byte]")
		} else {
			ecsLen := int(data[offset])
			offset++
			if len(data) < offset+ecsLen {
				parts = append(parts, "[ecs:incomplete_string]")
			} else {
				ecs := string(data[offset : offset+ecsLen])
				parts = append(parts, fmt.Sprintf("[ecs:%s]", ecs))
			}
		}
	}

	return strings.Join(parts, " ")
}

func dnsMsgToString(msg *dns.Msg) string {
	if msg == nil {
		return "<nil>\n"
	}
	return strings.TrimSpace(msg.String()) + "\n"
}

func (c *Cache) writeDump(w io.Writer) (int, error) {
	en := 0
	gw, _ := gzip.NewWriterLevel(w, gzip.BestSpeed)
	gw.Name = dumpHeader

	block := new(CacheDumpBlock)
	writeBlock := func() error {
		b, err := proto.Marshal(block)
		if err != nil {
			return fmt.Errorf("failed to marshal protobuf, %w", err)
		}
		l := make([]byte, 8)
		binary.BigEndian.PutUint64(l, uint64(len(b)))
		if _, err := gw.Write(l); err != nil {
			return fmt.Errorf("failed to write header, %w", err)
		}
		if _, err := gw.Write(b); err != nil {
			return fmt.Errorf("failed to write data, %w", err)
		}
		en += len(block.GetEntries())
		block.Reset()
		return nil
	}

	now := time.Now()
	rangeFunc := func(k key, v *item, cacheExpirationTime time.Time) error {
		if cacheExpirationTime.Before(now) {
			return nil
		}
		msg, err := v.resp.Pack()
		if err != nil {
			return fmt.Errorf("failed to pack msg, %w", err)
		}
		e := &CachedEntry{
			Key:                 []byte(k),
			CacheExpirationTime: cacheExpirationTime.Unix(),
			MsgExpirationTime:   v.expirationTime.Unix(),
			MsgStoredTime:       v.storedTime.Unix(),
			Msg:                 msg,
			DomainSet:           v.domainSet,
		}
		block.Entries = append(block.Entries, e)
		if len(block.Entries) >= dumpBlockSize {
			return writeBlock()
		}
		return nil
	}

	// [修改点] 遍历所有分片，汇总写入同一个文件
	for i := 0; i < shardCount; i++ {
		if err := c.shards[i].Range(rangeFunc); err != nil {
			return en, err
		}
	}

	if len(block.GetEntries()) > 0 {
		if err := writeBlock(); err != nil {
			return en, err
		}
	}
	return en, gw.Close()
}

func (c *Cache) readDump(r io.Reader) (int, error) {
	en := 0
	gr, err := gzip.NewReader(r)
	if err != nil {
		return en, fmt.Errorf("failed to read gzip header, %w", err)
	}
	if gr.Name != dumpHeader {
		return en, fmt.Errorf("invalid or old cache dump, header is %s, want %s", gr.Name, dumpHeader)
	}

	var errReadHeaderEOF = errors.New("")
	readBlock := func() error {
		// [恢复] 使用 pool.GetBuf 减少内存分配
		h := pool.GetBuf(8)
		defer pool.ReleaseBuf(h)
		_, err := io.ReadFull(gr, *h)
		if err != nil {
			if errors.Is(err, io.EOF) {
				return errReadHeaderEOF
			}
			return fmt.Errorf("failed to read block header, %w", err)
		}
		u := binary.BigEndian.Uint64(*h)
		if u > dumpMaximumBlockLength {
			return fmt.Errorf("invalid header, block length is big, %d", u)
		}
		b := pool.GetBuf(int(u))
		defer pool.ReleaseBuf(b)
		_, err = io.ReadFull(gr, *b)
		if err != nil {
			return fmt.Errorf("failed to read block data, %w", err)
		}
		block := new(CacheDumpBlock)
		if err := proto.Unmarshal(*b, block); err != nil {
			return fmt.Errorf("failed to decode block data, %w", err)
		}

		en += len(block.GetEntries())
		for _, entry := range block.GetEntries() {
			cacheExpTime := time.Unix(entry.GetCacheExpirationTime(), 0)
			msgExpTime := time.Unix(entry.GetMsgExpirationTime(), 0)
			storedTime := time.Unix(entry.GetMsgStoredTime(), 0)
			resp := new(dns.Msg)
			if err := resp.Unpack(entry.GetMsg()); err != nil {
				return fmt.Errorf("failed to decode dns msg, %w", err)
			}
			i := &item{
				resp:           resp,
				storedTime:     storedTime,
				expirationTime: msgExpTime,
				domainSet:      entry.GetDomainSet(),
			}
			// [修改点] 读取时重新计算分片，确保数据落入正确的桶
			// 这里的 key(entry.GetKey()) 转换成了 string 类型，然后 getShard 计算 Hash
			backend := c.getShard(string(entry.GetKey()))
			backend.Store(key(entry.GetKey()), i, cacheExpTime)
		}
		return nil
	}

	for {
		err = readBlock()
		if err != nil {
			if err == errReadHeaderEOF {
				err = nil
			}
			break
		}
	}

	if err != nil {
		return en, err
	}
	return en, gr.Close()
}
