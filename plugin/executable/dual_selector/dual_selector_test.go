package dual_selector

import (
	"context"
	"testing"
	"time"

	"github.com/IrineSistiana/mosdns/v5/coremain"
	"github.com/IrineSistiana/mosdns/v5/pkg/dnsutils"
	"github.com/IrineSistiana/mosdns/v5/pkg/query_context"
	"github.com/IrineSistiana/mosdns/v5/plugin/executable/sequence"
	"github.com/miekg/dns"
	"github.com/stretchr/testify/require"
)

func TestNormalizeExecErr_ExitWithResponseIsAccepted(t *testing.T) {
	q := new(dns.Msg)
	q.SetQuestion("example.com.", dns.TypeA)

	qCtx := query_context.NewContext(q)
	resp := new(dns.Msg)
	resp.SetReply(q)
	resp.Answer = append(resp.Answer, &dns.A{
		Hdr: dns.RR_Header{Name: "example.com.", Rrtype: dns.TypeA, Class: dns.ClassINET, Ttl: 60},
		A:   []byte{1, 1, 1, 1},
	})
	qCtx.SetResponse(resp)

	require.NoError(t, normalizeExecErr(sequence.ErrExit, qCtx))
}

func TestNormalizeExecErr_ExitWithoutResponseIsPreserved(t *testing.T) {
	q := new(dns.Msg)
	q.SetQuestion("example.com.", dns.TypeA)

	qCtx := query_context.NewContext(q)

	require.ErrorIs(t, normalizeExecErr(sequence.ErrExit, qCtx), sequence.ErrExit)
}

func TestPreferIpv4BlocksAAAAWhenReferenceACompletesLater(t *testing.T) {
	plugins := map[string]any{
		"answer_by_type": sequence.ExecutableFunc(func(_ context.Context, qCtx *query_context.Context) error {
			switch qCtx.Q().Question[0].Qtype {
			case dns.TypeA:
				time.Sleep(40 * time.Millisecond)
				resp := new(dns.Msg)
				resp.SetReply(qCtx.Q())
				resp.Answer = append(resp.Answer, &dns.A{
					Hdr: dns.RR_Header{Name: qCtx.Q().Question[0].Name, Rrtype: dns.TypeA, Class: dns.ClassINET, Ttl: 60},
					A:   []byte{1, 1, 1, 1},
				})
				qCtx.SetResponse(resp)
				return sequence.ErrExit
			case dns.TypeAAAA:
				resp := new(dns.Msg)
				resp.SetReply(qCtx.Q())
				resp.Answer = append(resp.Answer, &dns.AAAA{
					Hdr:  dns.RR_Header{Name: qCtx.Q().Question[0].Name, Rrtype: dns.TypeAAAA, Class: dns.ClassINET, Ttl: 60},
					AAAA: []byte{0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1},
				})
				qCtx.SetResponse(resp)
				return nil
			default:
				return nil
			}
		}),
	}

	m := coremain.NewTestMosdnsWithPlugins(plugins)
	s, err := sequence.NewSequence(coremain.NewBP("test", m), []sequence.RuleArgs{
		{Exec: "prefer_ipv4"},
		{Exec: "$answer_by_type"},
	})
	require.NoError(t, err)

	q := new(dns.Msg)
	q.SetQuestion("example.com.", dns.TypeAAAA)
	qCtx := query_context.NewContext(q)

	require.NoError(t, s.Exec(context.Background(), qCtx))
	require.NotNil(t, qCtx.R())
	require.Equal(t, 0, len(qCtx.R().Answer))
	require.Equal(t, dnsutils.GenEmptyReply(q, dns.RcodeSuccess).Rcode, qCtx.R().Rcode)
}
