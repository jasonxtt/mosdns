// Command m1a1-oracle instantiates the reference rule providers and
// domain_mapper from a configuration tree and answers a batch of queries, so
// the native Rust implementation can be compared against them line by line.
//
//	m1a1-oracle -root <config root> -config config_custom.yaml -in cases.jsonl -out answers.jsonl
//
// Each input line is a JSON object:
//
//	{"op":"rules","tag":"blocklist"}                  -> {"rules":[...],"sources":[...]}
//	{"op":"map","tag":"unified_matcher1","name":"a.example."}
//	                                                   -> {"flags":[...],"marks":[...],"domain_set":"","source":"","matched":bool}
//	{"op":"ip","tag":"geoip_cn","addr":"1.2.3.4"}     -> {"match":bool}
//	{"op":"qname","tag":"adguard","name":"a.example."}  -> {"match":bool}
//
// It only reads the configuration tree; run it on a copy.
package main

import (
	"bufio"
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"net/netip"
	"os"
	"path/filepath"
	"regexp"
	"runtime/debug"
	"sort"
	"strings"

	"github.com/IrineSistiana/mosdns/v5/coremain"
	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
	"github.com/IrineSistiana/mosdns/v5/pkg/utils"
	"github.com/IrineSistiana/mosdns/v5/plugin/data_provider"
	"github.com/IrineSistiana/mosdns/v5/plugin/data_provider/domain_mapper"
	"github.com/miekg/dns"
	"gopkg.in/yaml.v3"

	_ "github.com/IrineSistiana/mosdns/v5/plugin"
)

type pluginDef struct {
	Tag  string         `yaml:"tag"`
	Type string         `yaml:"type"`
	Args any `yaml:"args"`
}

type fileDef struct {
	Include []string    `yaml:"include"`
	Plugins []pluginDef `yaml:"plugins"`
}

func loadDefs(root, config string) ([]pluginDef, error) {
	read := func(path string) (*fileDef, error) {
		data, err := os.ReadFile(path)
		if err != nil {
			return nil, err
		}
		var f fileDef
		if err := yaml.Unmarshal(data, &f); err != nil {
			return nil, fmt.Errorf("%s: %w", path, err)
		}
		return &f, nil
	}
	mainPath := filepath.Join(root, config)
	main, err := read(mainPath)
	if err != nil {
		return nil, err
	}
	var defs []pluginDef
	for _, inc := range main.Include {
		f, err := read(filepath.Join(filepath.Dir(mainPath), inc))
		if err != nil {
			return nil, err
		}
		defs = append(defs, f.Plugins...)
	}
	return append(defs, main.Plugins...), nil
}

type request struct {
	Op   string `json:"op"`
	Tag  string `json:"tag"`
	Name string `json:"name"`
	Addr string `json:"addr"`
	// Pattern and Names drive the "regexp" op.
	Pattern string   `json:"pattern"`
	Names   []string `json:"names"`
}

func main() {
	root := flag.String("root", "", "configuration root (also the working directory)")
	config := flag.String("config", "config_custom.yaml", "main configuration file")
	in := flag.String("in", "", "input JSON lines")
	out := flag.String("out", "", "output JSON lines")
	flag.Parse()
	if *root == "" || *in == "" || *out == "" {
		flag.Usage()
		os.Exit(2)
	}
	if err := os.Chdir(*root); err != nil {
		fatal(err)
	}
	defs, err := loadDefs(".", *config)
	if err != nil {
		fatal(err)
	}

	plugins := map[string]any{}
	m := coremain.NewTestMosdnsWithPlugins(plugins)
	ctxMarks := map[uint32]struct{}{}
	build := func(def pluginDef) {
		info, ok := coremain.GetPluginType(def.Type)
		if !ok {
			fatal(fmt.Errorf("plugin type %s not defined", def.Type))
		}
		args := info.NewArgs()
		if err := utils.WeakDecode(def.Args, args); err != nil {
			fatal(fmt.Errorf("%s: %w", def.Tag, err))
		}
		if mapperArgs, ok := args.(*domain_mapper.Args); ok {
			if mapperArgs.DefaultCtxMark > 0 {
				ctxMarks[mapperArgs.DefaultCtxMark] = struct{}{}
			}
			for _, r := range mapperArgs.Rules {
				if r.CtxMark > 0 {
					ctxMarks[r.CtxMark] = struct{}{}
				}
			}
		}
		p, err := info.NewPlugin(coremain.NewBP(def.Tag, m), args)
		if err != nil {
			fatal(fmt.Errorf("%s (%s): %w", def.Tag, def.Type, err))
		}
		plugins[def.Tag] = p
	}
	providers := map[string]bool{"domain_set_light": true, "sd_set_light": true, "si_set": true, "adguard_rule": true}
	for _, def := range defs {
		if providers[def.Type] {
			build(def)
		}
	}
	for _, def := range defs {
		if def.Type == "domain_mapper" {
			build(def)
		}
	}
	var marks []uint32
	for mark := range ctxMarks {
		marks = append(marks, mark)
	}
	sort.Slice(marks, func(i, j int) bool { return marks[i] < marks[j] })

	reportMemory("after load")
	debug.FreeOSMemory()
	reportMemory("after GC")

	input, err := os.Open(*in)
	if err != nil {
		fatal(err)
	}
	defer input.Close()
	output, err := os.Create(*out)
	if err != nil {
		fatal(err)
	}
	defer output.Close()
	w := bufio.NewWriter(output)
	defer w.Flush()
	enc := json.NewEncoder(w)

	scanner := bufio.NewScanner(input)
	scanner.Buffer(make([]byte, 1<<20), 1<<24)
	for scanner.Scan() {
		var req request
		if err := json.Unmarshal(scanner.Bytes(), &req); err != nil {
			fatal(err)
		}
		plugin := plugins[req.Tag]
		switch req.Op {
		case "regexp":
			// The reference mapper/adguard use regexp.Compile and MatchString on
			// the normalized (lowercase, no trailing dot) name.
			re, err := regexp.Compile(req.Pattern)
			matches := make([]bool, len(req.Names))
			if err == nil {
				for i, name := range req.Names {
					matches[i] = re.MatchString(name)
				}
			}
			enc.Encode(map[string]any{"valid": err == nil, "matches": matches})
		case "rules":
			resp := map[string]any{}
			if d, ok := plugin.(data_provider.DetailedRuleExporter); ok {
				entries, _ := d.GetRuleEntries()
				rules := make([]string, len(entries))
				sources := make([]string, len(entries))
				for i, e := range entries {
					rules[i], sources[i] = e.Rule, e.SourceName
				}
				resp["rules"], resp["sources"] = rules, sources
			} else if e, ok := plugin.(data_provider.RuleExporter); ok {
				rules, _ := e.GetRules()
				resp["rules"] = rules
			}
			enc.Encode(resp)
		case "map":
			dm := plugin.(*domain_mapper.DomainMapper)
			q := new(dns.Msg)
			q.SetQuestion(req.Name, dns.TypeA)
			qCtx := query_context.NewContext(q)
			if err := dm.Exec(context.Background(), qCtx); err != nil {
				fatal(err)
			}
			flags := []int{}
			for bit := uint8(0); bit < 64; bit++ {
				if qCtx.HasFastFlag(bit) {
					flags = append(flags, int(bit))
				}
			}
			got := []uint32{}
			for _, mark := range marks {
				if qCtx.HasMark(mark) {
					got = append(got, mark)
				}
			}
			tags, _ := qCtx.GetValue(query_context.KeyDomainSet)
			sources, _ := qCtx.GetValue(query_context.KeyMatchedRuleSource)
			_, _, matched := dm.FastMatch(req.Name)
			enc.Encode(map[string]any{"flags": flags, "marks": got, "domain_set": tags, "source": sources, "matched": matched})
		case "qname":
			// The reference direct matcher (`qname $tag`).
			_, matched := plugin.(data_provider.DomainMatcherProvider).GetDomainMatcher().Match(req.Name)
			enc.Encode(map[string]any{"match": matched})
		case "count":
			// The reference mapper does not expose its rule count.
			enc.Encode(map[string]any{"rules": nil})
		case "ip":
			addr, err := netip.ParseAddr(req.Addr)
			if err != nil {
				fatal(err)
			}
			enc.Encode(map[string]any{"match": plugin.(data_provider.IPMatcherProvider).GetIPMatcher().Match(addr)})
		default:
			fatal(fmt.Errorf("unknown op %q", req.Op))
		}
	}
}

func fatal(err error) {
	fmt.Fprintln(os.Stderr, "m1a1-oracle:", err)
	os.Exit(1)
}

// reportMemory prints the process's resident and peak resident size to stderr.
func reportMemory(stage string) {
	status, err := os.ReadFile("/proc/self/status")
	if err != nil {
		return
	}
	for _, line := range strings.Split(string(status), "\n") {
		if strings.HasPrefix(line, "VmRSS") || strings.HasPrefix(line, "VmHWM") {
			fmt.Fprintf(os.Stderr, "m1a1-oracle %s: %s\n", stage, strings.Join(strings.Fields(line), " "))
		}
	}
}
