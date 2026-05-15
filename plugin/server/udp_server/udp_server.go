package udp_server

import (
	"context"
	"fmt"
	"net"

	"github.com/IrineSistiana/mosdns/v5/coremain"
	"github.com/IrineSistiana/mosdns/v5/pkg/server"
	"github.com/IrineSistiana/mosdns/v5/pkg/utils"
	"github.com/IrineSistiana/mosdns/v5/plugin/server/server_utils"
	"go.uber.org/zap"
)

const PluginType = "udp_server"

func init() {
	coremain.RegNewPluginFunc(PluginType, Init, func() any { return new(Args) })
}

type Args struct {
	Entry       string `yaml:"entry"`
	Listen      string `yaml:"listen"`
	EnableAudit bool   `yaml:"enable_audit"`
}

func (a *Args) init() {
	utils.SetDefaultString(&a.Listen, "127.0.0.1:53")
}

type UdpServer struct {
	args *Args
	c    net.PacketConn
}

func (s *UdpServer) Close() error {
	return s.c.Close()
}

func Init(bp *coremain.BP, args any) (any, error) {
	a := args.(*Args)
	a.init()
	return StartServer(bp, a)
}

func StartServer(bp *coremain.BP, args *Args) (*UdpServer, error) {
	handler, err := server_utils.NewHandler(bp, args.Entry, args.EnableAudit)
	if err != nil {
		return nil, fmt.Errorf("failed to init dns handler, %w", err)
	}

	socketOpt := server_utils.ListenerSocketOpts{
		SO_REUSEPORT: true,
		SO_RCVBUF:    2 * 1024 * 1024,
	}
	lc := net.ListenConfig{Control: server_utils.ListenerControl(socketOpt)}
	c, err := lc.ListenPacket(context.Background(), "udp", args.Listen)
	if err != nil {
		return nil, fmt.Errorf("failed to create socket, %w", err)
	}

	udpConn, ok := c.(*net.UDPConn)
	if !ok {
		_ = c.Close()
		return nil, fmt.Errorf("unexpected packet conn type %T", c)
	}

	bp.L().Info("udp server started", zap.Stringer("addr", c.LocalAddr()))

	go func() {
		defer c.Close()
		err := server.ServeUDP(udpConn, handler, server.UDPServerOpts{
			Logger: bp.L(),
		})
		bp.M().GetSafeClose().SendCloseSignal(err)
	}()

	return &UdpServer{args: args, c: c}, nil
}
