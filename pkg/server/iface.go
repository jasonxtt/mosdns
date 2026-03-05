package server

import (
	"context"
	"net/netip"

	"github.com/miekg/dns"
)

type Handler interface {
	Handle(ctx context.Context, q *dns.Msg, meta QueryMeta, packMsgPayload func(m *dns.Msg) (*[]byte, error)) (respPayload *[]byte)
}

type QueryMeta struct {
	FromUDP bool

	ClientAddr       netip.Addr
	ServerName       string
	UrlPath          string
	PreFastFlags     uint64
	PreFastDomainSet string
}
