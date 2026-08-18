package sequence

import (
	"context"
	"errors"
	"reflect"
	"testing"

	"github.com/IrineSistiana/mosdns/v5/coremain"
	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
	"github.com/miekg/dns"
)

type slice0Recorder struct {
	name string
	log  *[]string
	err  error
}

func (e *slice0Recorder) Exec(ctx context.Context, qCtx *query_context.Context, next ChainWalker) error {
	*e.log = append(*e.log, e.name)
	if e.err != nil {
		return e.err
	}
	return next.ExecNext(ctx, qCtx)
}

type slice0InlineCase struct {
	name          string
	inline        []interface{}
	target        []string
	targetFixture bool
	fixtureErr    error
	wantLog       []string
	wantErr       error
	wantResponse  bool
	wantRcode     int
}

func slice0ExecRules(execNames ...string) []RuleArgs {
	rules := make([]RuleArgs, 0, len(execNames))
	for _, name := range execNames {
		rules = append(rules, RuleArgs{Exec: name})
	}
	return rules
}

func TestSlice0InlineCharacterization(t *testing.T) {
	errInline := errors.New("inline executor error")
	errTry := errors.New("try ordinary error")
	cases := []slice0InlineCase{
		{
			name:    "multi-exec order then outer rule",
			inline:  []interface{}{"$exec1", "$exec2", "$exec3"},
			wantLog: []string{"exec1", "exec2", "exec3", "outer"},
		},
		{
			name:    "return ends inline only",
			inline:  []interface{}{"$exec1", "return", "$exec3"},
			wantLog: []string{"exec1", "outer"},
		},
		{
			name:    "accept ends inline only",
			inline:  []interface{}{"$exec1", "accept", "$exec3"},
			wantLog: []string{"exec1", "outer"},
		},
		{
			name:         "reject sets response and ends inline only",
			inline:       []interface{}{"$exec1", "reject", "$exec3"},
			wantLog:      []string{"exec1", "outer"},
			wantResponse: true,
			wantRcode:    dns.RcodeRefused,
		},
		{
			name:    "jump target fallthrough resumes inline",
			inline:  []interface{}{"$exec1", "jump target", "$exec3"},
			target:  []string{"$target-work"},
			wantLog: []string{"exec1", "target-work", "exec3", "outer"},
		},
		{
			name:    "jump target return resumes inline",
			inline:  []interface{}{"$exec1", "jump target", "$exec3"},
			target:  []string{"$target-work", "return"},
			wantLog: []string{"exec1", "target-work", "exec3", "outer"},
		},
		{
			name:    "goto replaces inline continuation",
			inline:  []interface{}{"$exec1", "goto target", "$exec3"},
			target:  []string{"$target-work"},
			wantLog: []string{"exec1", "target-work", "outer"},
		},
		{
			name:    "uncaught exit propagates from inline",
			inline:  []interface{}{"$exec1", "exit", "$exec3"},
			wantLog: []string{"exec1"},
			wantErr: ErrExit,
		},
		{
			name:    "try normal target continues inline",
			inline:  []interface{}{"$exec1", "try target", "$exec3"},
			target:  []string{"$target-work"},
			wantLog: []string{"exec1", "target-work", "exec3", "outer"},
		},
		{
			name:    "try target exit is caught",
			inline:  []interface{}{"$exec1", "try target", "$exec3"},
			target:  []string{"exit"},
			wantLog: []string{"exec1", "exec3", "outer"},
		},
		{
			name:          "try fixture exit is caught",
			inline:        []interface{}{"$exec1", "try $fixture", "$exec3"},
			targetFixture: true,
			wantLog:       []string{"exec1", "fixture", "exec3", "outer"},
		},
		{
			name:          "try fixture ordinary error propagates",
			inline:        []interface{}{"$exec1", "try $fixture", "$exec3"},
			targetFixture: true,
			fixtureErr:    errTry,
			wantLog:       []string{"exec1", "fixture"},
			wantErr:       errTry,
		},
		{
			name:    "ordinary inline error propagates",
			inline:  []interface{}{"$exec1", "$inline-error", "$exec3"},
			wantLog: []string{"exec1", "inline-error"},
			wantErr: errInline,
		},
	}

	for _, tt := range cases {
		t.Run(tt.name, func(t *testing.T) {
			log := make([]string, 0, len(tt.wantLog))
			plugins := make(map[string]any)
			mosdns := coremain.NewTestMosdnsWithPlugins(plugins)

			for _, name := range []string{"exec1", "exec2", "exec3", "outer", "target-work"} {
				plugins[name] = &slice0Recorder{name: name, log: &log}
			}
			plugins["inline-error"] = &slice0Recorder{name: "inline-error", log: &log, err: errInline}
			if tt.targetFixture {
				fixtureErr := tt.fixtureErr
				if fixtureErr == nil {
					fixtureErr = ErrExit
				}
				plugins["fixture"] = RecursiveExecutableFunc(func(_ context.Context, _ *query_context.Context, _ ChainWalker) error {
					log = append(log, "fixture")
					return fixtureErr
				})
			}

			if len(tt.target) > 0 {
				target, err := NewSequence(coremain.NewBP("slice0-target", mosdns), slice0ExecRules(tt.target...))
				if err != nil {
					t.Fatal(err)
				}
				plugins["target"] = target
			}

			mainRules := []RuleArgs{
				{Exec: tt.inline},
				{Exec: "$outer"},
			}
			sequence, err := NewSequence(coremain.NewBP("slice0-main", mosdns), mainRules)
			if err != nil {
				t.Fatal(err)
			}

			qCtx := query_context.NewContext(new(dns.Msg))
			err = sequence.Exec(context.Background(), qCtx)
			if tt.wantErr != nil {
				if !errors.Is(err, tt.wantErr) {
					t.Fatalf("Exec() error = %v, want errors.Is(..., %v)", err, tt.wantErr)
				}
			} else if err != nil {
				t.Fatalf("Exec() error = %v, want nil", err)
			}

			if !reflect.DeepEqual(log, tt.wantLog) {
				t.Errorf("execution log = %#v, want %#v", log, tt.wantLog)
			}
			response := qCtx.R()
			if (response != nil) != tt.wantResponse {
				t.Errorf("response present = %v, want %v", response != nil, tt.wantResponse)
			}
			if response != nil && response.Rcode != tt.wantRcode {
				t.Errorf("response RCODE = %d, want %d", response.Rcode, tt.wantRcode)
			}
		})
	}
}
