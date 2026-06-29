// switcher17.go
package switcher17

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

const PluginType = "switch17"
const switchBit = 49

type Args struct {
	InitialValue string `yaml:"initial_value"`
}

type Switcher17 struct {
	value    atomic.Value
	filePath string
	writeMu  sync.Mutex
}

var globalSwitcher17 *Switcher17

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
	sw := &Switcher17{filePath: cfg.InitialValue}

	if err := os.MkdirAll(filepath.Dir(sw.filePath), 0o755); err != nil {
		return nil, fmt.Errorf("cannot create dir for %s file: %w", PluginType, err)
	}

	var initVal string
	data, err := os.ReadFile(sw.filePath)
	if err == nil {
		initVal = strings.TrimSpace(string(data))
	} else {
		initVal = ""
		_ = os.WriteFile(sw.filePath, []byte(initVal), 0o644)
	}

	sw.value.Store(initVal)
	updateGlobalMask(initVal)

	globalSwitcher17 = sw
	bp.RegAPI(sw.Api())
	return sw, nil
}

func updateGlobalMask(val string) {
	mask := query_context.GlobalSwitchMask.Load()
	if val == "A" {
		mask |= (1 << switchBit)
	} else {
		mask &^= (1 << switchBit)
	}
	query_context.GlobalSwitchMask.Store(mask)
}

func (s *Switcher17) Exec(ctx context.Context, qCtx *query_context.Context, next sequence.ChainWalker) error {
	return next.ExecNext(ctx, qCtx)
}

func (s *Switcher17) Api() *chi.Mux {
	r := chi.NewRouter()

	r.Get("/show", func(w http.ResponseWriter, r *http.Request) {
		val := s.value.Load().(string)
		_, _ = io.WriteString(w, val)
	})

	r.Post("/post", func(w http.ResponseWriter, r *http.Request) {
		var newVal string
		if strings.HasPrefix(r.Header.Get("Content-Type"), "application/json") {
			var body struct {
				Value string `json:"value"`
			}
			if err := json.NewDecoder(r.Body).Decode(&body); err == nil {
				newVal = body.Value
			}
		}
		if newVal == "" {
			_ = r.ParseForm()
			newVal = r.FormValue("value")
		}

		s.writeMu.Lock()
		defer s.writeMu.Unlock()

		s.value.Store(newVal)
		updateGlobalMask(newVal)

		if err := os.WriteFile(s.filePath, []byte(newVal), 0o644); err != nil {
			http.Error(w, "failed to write switch file: "+err.Error(), http.StatusInternalServerError)
			return
		}

		w.WriteHeader(http.StatusOK)
		_, _ = fmt.Fprintf(w, "updated to: %s\n", newVal)
	})

	return r
}

func QuickSetup(_ sequence.BQ, raw string) (sequence.Matcher, error) {
	expected := strings.Trim(raw, `"'`)
	return &switchMatcher17{expected: expected}, nil
}

type switchMatcher17 struct {
	expected string
}

func (m *switchMatcher17) Match(_ context.Context, qCtx *query_context.Context) (bool, error) {
	if m.expected == "A" {
		return qCtx.HasFastFlag(switchBit), nil
	}

	if globalSwitcher17 == nil {
		return false, nil
	}
	currentVal := globalSwitcher17.value.Load().(string)
	return currentVal == m.expected, nil
}

func (m *switchMatcher17) GetFastCheck() func(qCtx *query_context.Context) bool {
	exp := m.expected
	return func(qCtx *query_context.Context) bool {
		if exp == "A" {
			return qCtx.HasFastFlag(switchBit)
		}
		if globalSwitcher17 == nil {
			return false
		}
		v := globalSwitcher17.value.Load().(string)
		return v == exp
	}
}

func (s *Switcher17) GetValue() string {
	if val, ok := s.value.Load().(string); ok {
		return val
	}
	return ""
}
