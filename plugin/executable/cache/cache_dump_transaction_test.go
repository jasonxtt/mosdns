package cache

import (
	"bytes"
	"compress/gzip"
	"encoding/binary"
	"testing"
	"time"

	"github.com/miekg/dns"
	"google.golang.org/protobuf/proto"
)

func dumpEntryFixture(t *testing.T, entryKey string) *CachedEntry {
	t.Helper()
	q := new(dns.Msg)
	q.SetQuestion("dump.example.", dns.TypeA)
	r := new(dns.Msg)
	r.SetReply(q)
	r.Answer = []dns.RR{mustRR(t, "dump.example. 60 IN A 192.0.2.9")}
	wire, err := r.Pack()
	if err != nil {
		t.Fatal(err)
	}
	now := time.Now().Truncate(time.Second)
	return &CachedEntry{
		Key:                 []byte(entryKey),
		Msg:                 wire,
		MsgStoredTime:       now.Unix(),
		MsgExpirationTime:   now.Add(time.Minute).Unix(),
		CacheExpirationTime: now.Add(time.Minute).Unix(),
		DomainSet:           "dump-transaction",
	}
}

func dumpPayload(t *testing.T, entry *CachedEntry, truncatedTail bool) []byte {
	t.Helper()
	var payload bytes.Buffer
	gw := gzip.NewWriter(&payload)
	gw.Name = dumpHeader
	block, err := proto.Marshal(&CacheDumpBlock{Entries: []*CachedEntry{entry}})
	if err != nil {
		t.Fatal(err)
	}
	var header [8]byte
	binary.BigEndian.PutUint64(header[:], uint64(len(block)))
	if _, err := gw.Write(header[:]); err != nil {
		t.Fatal(err)
	}
	if _, err := gw.Write(block); err != nil {
		t.Fatal(err)
	}
	if truncatedTail {
		binary.BigEndian.PutUint64(header[:], 10)
		if _, err := gw.Write(header[:]); err != nil {
			t.Fatal(err)
		}
		if _, err := gw.Write([]byte{1, 2, 3}); err != nil {
			t.Fatal(err)
		}
	}
	if err := gw.Close(); err != nil {
		t.Fatal(err)
	}
	return payload.Bytes()
}

func TestReadDumpDoesNotPartiallyImportMalformedPayload(t *testing.T) {
	c := NewCache(&Args{Size: 16}, Opts{})
	t.Cleanup(func() { _ = c.Close() })
	existing := dumpEntryFixture(t, "existing")
	c.backend.Store(key(existing.GetKey()), &item{
		resp:           append([]byte(nil), existing.GetMsg()...),
		storedTime:     time.Unix(existing.GetMsgStoredTime(), 0),
		expirationTime: time.Unix(existing.GetMsgExpirationTime(), 0),
		domainSet:      "existing",
	}, time.Unix(existing.GetCacheExpirationTime(), 0))

	payload := dumpPayload(t, dumpEntryFixture(t, "must-not-import"), true)
	if _, err := c.readDump(bytes.NewReader(payload)); err == nil {
		t.Fatal("truncated dump was accepted")
	}
	if _, _, ok := c.backend.Get(key("must-not-import")); ok {
		t.Fatal("entry preceding malformed tail was partially imported")
	}
	if got, _, ok := c.backend.Get(key("existing")); !ok || got.domainSet != "existing" {
		t.Fatalf("existing cache entry changed: present=%v item=%+v", ok, got)
	}
}

func TestReadDumpRejectsMalformedDNSWireBeforeMutation(t *testing.T) {
	c := NewCache(&Args{Size: 16}, Opts{})
	t.Cleanup(func() { _ = c.Close() })
	entry := dumpEntryFixture(t, "invalid-wire")
	entry.Msg = []byte{1, 2, 3}

	if _, err := c.readDump(bytes.NewReader(dumpPayload(t, entry, false))); err == nil {
		t.Fatal("malformed DNS wire was accepted")
	}
	if c.backend.Len() != 0 {
		t.Fatalf("malformed dump mutated cache: len=%d", c.backend.Len())
	}
}

func TestReadDumpMirrorsValidatedEntriesToActiveRustBackend(t *testing.T) {
	originalFactory := rustBackendFactory
	t.Cleanup(func() { rustBackendFactory = originalFactory })
	t.Setenv(rustCacheBackendEnv, "rust")
	fake := new(fakeRustCacheBackend)
	rustBackendFactory = func(*Args) (rustCacheBackend, error) { return fake, nil }
	c := NewCache(&Args{Size: 16}, Opts{})
	t.Cleanup(func() { _ = c.Close() })

	payload := dumpPayload(t, dumpEntryFixture(t, "rust-import"), false)
	if entries, err := c.readDump(bytes.NewReader(payload)); err != nil || entries != 1 {
		t.Fatalf("readDump entries=%d err=%v", entries, err)
	}
	if fake.stores != 1 {
		t.Fatalf("Rust mirror stores=%d, want 1", fake.stores)
	}
}
