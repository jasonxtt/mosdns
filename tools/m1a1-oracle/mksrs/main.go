// Command mksrs writes the small SRS fixtures used by the native provider
// tests, built with the same sing library the reference providers read with.
//
//	mksrs <output dir>
package main

import (
	"bufio"
	"bytes"
	"compress/zlib"
	"encoding/binary"
	"fmt"
	"net/netip"
	"os"
	"path/filepath"

	"github.com/IrineSistiana/mosdns/v5/plugin/data_provider/testutil"
	"github.com/sagernet/sing/common/varbin"
)

func ipSRS(ranges [][2]netip.Addr) ([]byte, error) {
	var compressed bytes.Buffer
	zw := zlib.NewWriter(&compressed)
	bw := bufio.NewWriter(zw)
	var tmp [binary.MaxVarintLen64]byte
	bw.Write(tmp[:binary.PutUvarint(tmp[:], 1)]) // one rule
	bw.WriteByte(0)                               // default rule
	bw.WriteByte(6)                               // ip cidr item
	bw.WriteByte(1)                               // ip set version
	binary.Write(bw, binary.BigEndian, uint64(len(ranges)))
	for _, r := range ranges {
		if err := varbin.Write(bw, binary.BigEndian, r[0].AsSlice()); err != nil {
			return nil, err
		}
		if err := varbin.Write(bw, binary.BigEndian, r[1].AsSlice()); err != nil {
			return nil, err
		}
	}
	bw.WriteByte(0xFF)
	if err := bw.Flush(); err != nil {
		return nil, err
	}
	if err := zw.Close(); err != nil {
		return nil, err
	}
	return append([]byte{'S', 'R', 'S', 3}, compressed.Bytes()...), nil
}

func main() {
	if len(os.Args) != 2 {
		fmt.Fprintln(os.Stderr, "usage: mksrs <output dir>")
		os.Exit(2)
	}
	out := os.Args[1]
	domains, err := testutil.BuildDomainSRS(
		[]string{"exact.srs.example", "other.srs.example"},
		[]string{"suffix.srs.example", "example.cn"},
		[]string{"kwsrs"},
		[]string{`^re[0-9]+\.srs\.example$`},
	)
	must(err)
	must(os.WriteFile(filepath.Join(out, "domains.srs"), domains, 0o644))
	ips, err := ipSRS([][2]netip.Addr{
		{netip.MustParseAddr("10.0.0.0"), netip.MustParseAddr("10.0.0.255")},
		{netip.MustParseAddr("192.0.2.5"), netip.MustParseAddr("192.0.2.9")},
		{netip.MustParseAddr("2001:db8::"), netip.MustParseAddr("2001:db8::ffff")},
	})
	must(err)
	must(os.WriteFile(filepath.Join(out, "ips.srs"), ips, 0o644))
}

func must(err error) {
	if err != nil {
		fmt.Fprintln(os.Stderr, "mksrs:", err)
		os.Exit(1)
	}
}
