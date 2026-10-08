// Command m1c1-interop drives the reference domain_output plugin so files
// written by the native host can be read back by the reference code and vice
// versa (M1-c1 interop evidence).
//
//	m1c1-interop -root <dir> -stat gen/top.txt [-rule gen/rule.txt] [-flags] \
//	    [-record name[,name...]] [-record-flags AD,CD,DO] -method GET -path /show?limit=5
//
// The plugin is created with the working directory set to <dir>, so it loads
// <stat> as it would at start. Each -record name is executed once as a query
// (optionally with the AD, CD and DO flags set), then the HTTP request is
// served by the plugin's own router; the status, the X-Total-Count header and
// the body are printed. "-path /save" makes the reference write its files.
package main

import (
	"flag"
	"fmt"
	"net/http/httptest"
	"os"
	"strings"
	"time"

	"github.com/IrineSistiana/mosdns/v5/coremain"
	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
	"github.com/IrineSistiana/mosdns/v5/pkg/utils"
	"github.com/IrineSistiana/mosdns/v5/plugin/executable/sequence"
	"github.com/miekg/dns"

	_ "github.com/IrineSistiana/mosdns/v5/plugin"
)

func main() {
	root := flag.String("root", "", "configuration root (working directory)")
	stat := flag.String("stat", "", "file_stat")
	rule := flag.String("rule", "", "file_rule")
	flags := flag.Bool("flags", false, "enable_flags")
	record := flag.String("record", "", "comma separated query names to execute")
	recordFlags := flag.String("record-flags", "", "flags of the recorded queries: AD,CD,DO")
	method := flag.String("method", "GET", "HTTP method")
	path := flag.String("path", "/show", "path (with query) below /plugins/<tag>")
	flag.Parse()
	if *root == "" || *stat == "" {
		flag.Usage()
		os.Exit(2)
	}
	if err := os.Chdir(*root); err != nil {
		fatal(err)
	}
	info, ok := coremain.GetPluginType("domain_output")
	if !ok {
		fatal(fmt.Errorf("plugin type domain_output not defined"))
	}
	args := info.NewArgs()
	cfg := map[string]any{"file_stat": *stat, "enable_flags": *flags, "dump_interval": 3600}
	if *rule != "" {
		cfg["file_rule"] = *rule
	}
	if err := utils.WeakDecode(cfg, args); err != nil {
		fatal(err)
	}
	m := coremain.NewTestMosdnsWithPlugins(map[string]any{})
	plugin, err := info.NewPlugin(coremain.NewBP("t", m), args)
	if err != nil {
		fatal(err)
	}
	if *record != "" {
		exec := plugin.(sequence.Executable)
		for _, name := range strings.Split(*record, ",") {
			msg := new(dns.Msg)
			msg.SetQuestion(dns.Fqdn(name), dns.TypeA)
			for _, flag := range strings.Split(*recordFlags, ",") {
				switch flag {
				case "AD":
					msg.AuthenticatedData = true
				case "CD":
					msg.CheckingDisabled = true
				case "DO":
					msg.SetEdns0(1232, true)
				}
			}
			if err := exec.Exec(nil, query_context.NewContext(msg)); err != nil {
				fatal(err)
			}
		}
		// The reference counts on a background goroutine.
		time.Sleep(300 * time.Millisecond)
	}
	request := httptest.NewRequest(*method, "/plugins/t"+*path, nil)
	recorder := httptest.NewRecorder()
	m.GetAPIRouter().ServeHTTP(recorder, request)
	fmt.Println(recorder.Code)
	fmt.Println("X-Total-Count:", recorder.Header().Get("X-Total-Count"))
	fmt.Print(recorder.Body.String())
}

func fatal(err error) {
	fmt.Fprintln(os.Stderr, err)
	os.Exit(1)
}
