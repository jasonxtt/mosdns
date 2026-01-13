// switcher12.go
package switcher12

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"sync/atomic"

	"github.com/IrineSistiana/mosdns/v5/coremain"
	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
	"github.com/IrineSistiana/mosdns/v5/plugin/executable/sequence"
	"github.com/go-chi/chi/v5"
)

const PluginType = "switch12"

type Args struct {
	InitialValue string `yaml:"initial_value"`
}

type Switcher12 struct {
	value    atomic.Value
	filePath string
	writeMu  sync.Mutex
}

var globalSwitcher12 *Switcher12

func init() {
	sequence.MustRegMatchQuickSetup(PluginType, QuickSetup)
	coremain.RegNewPluginFunc(
		PluginType,
		Init,
		func() any { return new(Args) },
	)
}

func Init(bp *coremain.BP, args any) (any, error) {
	cfg := args.(*Args)
	sw := &Switcher12{filePath: cfg.InitialValue}

	if err := os.MkdirAll(filepath.Dir(sw.filePath), 0755); err != nil {
		return nil, fmt.Errorf("cannot create dir for %s file: %w", PluginType, err)
	}

	var initVal string
	data, err := os.ReadFile(sw.filePath)
	if err == nil {
		initVal = strings.TrimSpace(string(data))
	} else {
		initVal = ""
		_ = os.WriteFile(sw.filePath, []byte(initVal), 0644)
	}

	sw.value.Store(initVal)

	globalSwitcher12 = sw
	bp.RegAPI(sw.Api())
	return sw, nil
}

func (s *Switcher12) Exec(ctx context.Context, qCtx *query_context.Context, next sequence.ChainWalker) error {
	return next.ExecNext(ctx, qCtx)
}

func (s *Switcher12) Api() *chi.Mux {
	r := chi.NewRouter()

	r.Get("/show", func(w http.ResponseWriter, r *http.Request) {
		val := s.value.Load().(string)
		io.WriteString(w, val)
	})

	r.Post("/post", func(w http.ResponseWriter, r *http.Request) {
		var newVal string
		if strings.HasPrefix(r.Header.Get("Content-Type"), "application/json") {
			var body struct{ Value string }
			if err := json.NewDecoder(r.Body).Decode(&body); err == nil {
				newVal = body.Value
			}
		}
		if newVal == "" {
			r.ParseForm()
			newVal = r.FormValue("value")
		}

		s.writeMu.Lock()
		defer s.writeMu.Unlock()

		s.value.Store(newVal)

		if err := os.WriteFile(s.filePath, []byte(newVal), 0644); err != nil {
			http.Error(w, "failed to write switch file: "+err.Error(), http.StatusInternalServerError)
			return
		}

		w.WriteHeader(http.StatusOK)
		fmt.Fprintf(w, "updated to: %s\n", newVal)
	})

	return r
}

func QuickSetup(_ sequence.BQ, raw string) (sequence.Matcher, error) {
	expected := strings.Trim(raw, `"'`)
	return &switchMatcher12{expected: expected}, nil
}

type switchMatcher12 struct {
	expected string
}

func (m *switchMatcher12) Match(_ context.Context, _ *query_context.Context) (bool, error) {
	if globalSwitcher12 == nil {
		return false, nil
	}
	currentVal := globalSwitcher12.value.Load().(string)
	return currentVal == m.expected, nil
}
