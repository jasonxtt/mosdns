// Command m1a2-interop drives the reference rule-source providers through
// their own HTTP handlers, so manifests written by the native host can be read
// back by the reference code and vice versa (M1-a2 interop evidence).
//
//	m1a2-interop -root <dir> -type sd_set_light|si_set|adguard_rule -tag <tag> -arg <local_config|dir> \
//	    -method GET|PUT|POST|DELETE -path /config -body '<json>'
//
// It prints the HTTP status on the first line and the body after it. The
// provider is instantiated with the working directory set to <dir>; any
// download it starts in the background is abandoned when the command exits.
package main

import (
	"flag"
	"fmt"
	"net/http/httptest"
	"os"
	"strings"

	"github.com/IrineSistiana/mosdns/v5/coremain"
	"github.com/IrineSistiana/mosdns/v5/pkg/utils"

	_ "github.com/IrineSistiana/mosdns/v5/plugin"
)

func main() {
	root := flag.String("root", "", "configuration root (working directory)")
	typ := flag.String("type", "", "plugin type")
	tag := flag.String("tag", "", "plugin tag")
	arg := flag.String("arg", "", "local_config (sd_set_light, si_set) or dir (adguard_rule)")
	method := flag.String("method", "GET", "HTTP method")
	path := flag.String("path", "/config", "path below /plugins/<tag>")
	body := flag.String("body", "", "request body")
	flag.Parse()
	if *root == "" || *typ == "" || *tag == "" || *arg == "" {
		flag.Usage()
		os.Exit(2)
	}
	if err := os.Chdir(*root); err != nil {
		fatal(err)
	}
	info, ok := coremain.GetPluginType(*typ)
	if !ok {
		fatal(fmt.Errorf("plugin type %s not defined", *typ))
	}
	key := "local_config"
	if *typ == "adguard_rule" {
		key = "dir"
	}
	args := info.NewArgs()
	if err := utils.WeakDecode(map[string]any{key: *arg}, args); err != nil {
		fatal(err)
	}
	m := coremain.NewTestMosdnsWithPlugins(map[string]any{})
	if _, err := info.NewPlugin(coremain.NewBP(*tag, m), args); err != nil {
		fatal(err)
	}
	request := httptest.NewRequest(*method, "/plugins/"+*tag+*path, strings.NewReader(*body))
	recorder := httptest.NewRecorder()
	m.GetAPIRouter().ServeHTTP(recorder, request)
	fmt.Println(recorder.Code)
	fmt.Print(recorder.Body.String())
}

func fatal(err error) {
	fmt.Fprintln(os.Stderr, err)
	os.Exit(1)
}
