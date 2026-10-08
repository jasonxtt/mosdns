// Command m1c2-interop drives the reference requery plugin through its own
// HTTP handlers, so a state file written by the native host can be read back
// by the reference code and vice versa (M1-c2 interop evidence).
//
//	m1c2-interop -root <dir> -file webinfo/requeryconfig.json \
//	    -method GET|POST -path / [-body '<json>']
//
// The plugin is created with the working directory set to <dir>, so it loads
// (and, like the reference at start, normalises and rewrites) the file. The
// HTTP status and body are printed. Nothing is triggered unless asked to.
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
	file := flag.String("file", "", "requery state file")
	method := flag.String("method", "GET", "HTTP method")
	path := flag.String("path", "/", "path below /plugins/requery")
	body := flag.String("body", "", "request body")
	flag.Parse()
	if *root == "" || *file == "" {
		flag.Usage()
		os.Exit(2)
	}
	if err := os.Chdir(*root); err != nil {
		fatal(err)
	}
	info, ok := coremain.GetPluginType("requery")
	if !ok {
		fatal(fmt.Errorf("plugin type requery not defined"))
	}
	args := info.NewArgs()
	if err := utils.WeakDecode(map[string]any{"file": *file}, args); err != nil {
		fatal(err)
	}
	m := coremain.NewTestMosdnsWithPlugins(map[string]any{})
	if _, err := info.NewPlugin(coremain.NewBP("requery", m), args); err != nil {
		fatal(err)
	}
	request := httptest.NewRequest(*method, "/plugins/requery"+*path, strings.NewReader(*body))
	recorder := httptest.NewRecorder()
	m.GetAPIRouter().ServeHTTP(recorder, request)
	fmt.Println(recorder.Code)
	fmt.Print(recorder.Body.String())
}

func fatal(err error) {
	fmt.Fprintln(os.Stderr, err)
	os.Exit(1)
}
