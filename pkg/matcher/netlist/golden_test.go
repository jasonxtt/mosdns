/*
 * Copyright (C) 2020-2026, IrineSistiana
 *
 * Slice 0 golden fixtures for the Go netlist matcher. These vectors freeze the
 * exact Go behavior (IPv4->16-byte mapping, masking, overlap collapse, binary
 * search containment) that the Rust IP core (Slice 3) must reproduce. Do not
 * weaken them to make an implementation pass.
 */

package netlist

import (
	"net/netip"
	"strings"
	"testing"
)

func TestGoldenListMaskingAndMapping(t *testing.T) {
	raw := `
# comment stripped
10.0.0.0/8
192.168.1.1        # host address -> /32
2001:db8::/32
2001:db9:beef::1   # ipv6 host -> /128, outside the db8::/32 prefix
1.2.3.0/24
`
	l := NewList()
	if err := LoadFromReader(l, strings.NewReader(raw)); err != nil {
		t.Fatal(err)
	}
	l.Sort()

	tests := []struct {
		name string
		addr string
		want bool
	}{
		{"v4 inside", "10.1.2.3", true},
		{"v4 outside", "11.0.0.1", false},
		{"v4 host exact", "192.168.1.1", true},
		{"v4 host neighbor", "192.168.1.2", false},
		{"v6 inside", "2001:db8:1::1", true},
		{"v6 outside", "2001:dbe::1", false},
		{"v6 host exact", "2001:db9:beef::1", true},
		{"v6 host neighbor", "2001:db9:beef::2", false},
		{"v6 prefix covers deeper address", "2001:db8:beef::2", true},
		{"v4 mapped matches v4 prefix", "::ffff:1.2.3.4", true},
		{"v4 mapped outside", "::ffff:1.2.4.4", false},
		{"plain v4 inside mapped prefix", "1.2.3.4", true},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			addr := netip.MustParseAddr(tt.addr)
			if got := l.Contains(addr); got != tt.want {
				t.Errorf("Contains(%s) = %v, want %v", tt.addr, got, tt.want)
			}
		})
	}
}

func TestGoldenListOverlapCollapse(t *testing.T) {
	t.Run("nested prefix folded", func(t *testing.T) {
		l := NewList()
		for _, s := range []string{"192.168.0.0/16", "192.168.1.0/24", "192.168.9.0/24"} {
			p := netip.MustParsePrefix(s)
			l.Append(p)
		}
		l.Sort()
		if l.Len() != 1 {
			t.Fatalf("Len() = %d, want 1", l.Len())
		}
		if !l.Contains(netip.MustParseAddr("192.168.255.255")) {
			t.Fatal("folded /16 must still contain deep address")
		}
	})
	t.Run("same address keeps smaller bits", func(t *testing.T) {
		l := NewList()
		for _, s := range []string{"192.168.0.0/24", "192.168.0.0/16"} {
			l.Append(netip.MustParsePrefix(s))
		}
		l.Sort()
		if l.Len() != 1 {
			t.Fatalf("Len() = %d, want 1", l.Len())
		}
		if !l.Contains(netip.MustParseAddr("192.168.9.9")) {
			t.Fatal("kept /16 must contain 192.168.9.9")
		}
	})
	t.Run("disjoint prefixes kept", func(t *testing.T) {
		l := NewList()
		for _, s := range []string{"1.0.0.0/8", "2.0.0.0/8", "3.0.0.0/8"} {
			l.Append(netip.MustParsePrefix(s))
		}
		l.Sort()
		if l.Len() != 3 {
			t.Fatalf("Len() = %d, want 3", l.Len())
		}
	})
}

func TestGoldenListContainmentBoundaries(t *testing.T) {
	l := NewList()
	for _, s := range []string{"192.168.0.0/24", "10.0.0.0/8"} {
		l.Append(netip.MustParsePrefix(s))
	}
	l.Sort()

	tests := []struct {
		addr netip.Addr
		want bool
	}{
		{netip.MustParseAddr("192.168.0.0"), true},
		{netip.MustParseAddr("192.168.0.255"), true},
		{netip.MustParseAddr("192.168.1.0"), false},
		{netip.MustParseAddr("192.167.255.255"), false},
		{netip.MustParseAddr("10.0.0.0"), true},
		{netip.MustParseAddr("10.255.255.255"), true},
		{netip.MustParseAddr("11.0.0.0"), false},
		{netip.Addr{}, false}, // invalid address
	}
	for i, tt := range tests {
		if got := l.Contains(tt.addr); got != tt.want {
			t.Errorf("#%d Contains(%v) = %v, want %v", i, tt.addr, got, tt.want)
		}
	}
}

func TestGoldenListInvalidInput(t *testing.T) {
	t.Run("invalid prefix string rejected", func(t *testing.T) {
		for _, bad := range []string{"300.1.2.3", "1.2.3.0/99", "not-an-ip", ":::"} {
			if err := LoadFromText(NewList(), bad); err == nil {
				t.Errorf("LoadFromText(%q) must fail", bad)
			}
		}
	})
	t.Run("empty list returns false", func(t *testing.T) {
		l := NewList()
		l.Sort()
		if l.Contains(netip.MustParseAddr("1.2.3.4")) {
			t.Fatal("empty list must not contain anything")
		}
	})
	t.Run("unsorted list panics on Contains", func(t *testing.T) {
		l := NewList()
		l.Append(netip.MustParsePrefix("1.2.3.0/24"))
		defer func() {
			if recover() == nil {
				t.Fatal("Contains on unsorted list must panic")
			}
		}()
		l.Contains(netip.MustParseAddr("1.2.3.4"))
	})
}
