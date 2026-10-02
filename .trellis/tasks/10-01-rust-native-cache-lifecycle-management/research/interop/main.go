// Isolated persistence interoperability proof. Uses the repository's generated
// dump.pb.go and the actual product DNS/protobuf libraries; no Go runtime fallback.
package main
import (
 "bytes"
 "compress/gzip"
 "encoding/binary"
 "fmt"
 "io"
 "os"
 "github.com/miekg/dns"
 "google.golang.org/protobuf/proto"
 cache "cache-interop/pb"
)
func main() {
 if len(os.Args) != 3 { panic("generate|verify path") }
 path := os.Args[2]
 if os.Args[1] == "generate" {
  message := new(dns.Msg); message.SetQuestion("A\\.\\000.", dns.TypeA); message.Response = true
  rr, err := dns.NewRR("A\\.\\000. 60 IN A 192.0.2.7"); if err != nil { panic(err) }; message.Answer = []dns.RR{rr}
  wire, err := message.Pack(); if err != nil { panic(err) }
  key := []byte{7, 0, 1, byte(len(message.Question[0].Name))}; key = append(key, message.Question[0].Name...)
  block := &cache.CacheDumpBlock{Entries: []*cache.CachedEntry{{Key: key, Msg: wire, MsgStoredTime: 1700000000, MsgExpirationTime: 1700000060, CacheExpirationTime: 1700000090, DomainSet: "go-domain"}}}
  payload, err := proto.Marshal(block); if err != nil { panic(err) }
  var buf bytes.Buffer; writer := gzip.NewWriter(&buf); writer.Name = "mosdns_cache_v2"
  var framing [8]byte; binary.BigEndian.PutUint64(framing[:], uint64(len(payload))); writer.Write(framing[:]); writer.Write(payload); if err := writer.Close(); err != nil { panic(err) }
  if err := os.WriteFile(path, buf.Bytes(), 0600); err != nil { panic(err) }; fmt.Println("Go generated eligible v2 fixture")
 } else {
  file, err := os.Open(path); if err != nil { panic(err) }; defer file.Close()
  reader, err := gzip.NewReader(file); if err != nil { panic(err) }; if reader.Name != "mosdns_cache_v2" { panic("wrong Name") }
  count := 0
  for { var size [8]byte; _, err := io.ReadFull(reader, size[:]); if err == io.EOF { break }; if err != nil { panic(err) }
   payload := make([]byte, binary.BigEndian.Uint64(size[:])); if _, err := io.ReadFull(reader, payload); err != nil { panic(err) }
   block := new(cache.CacheDumpBlock); if err := proto.Unmarshal(payload, block); err != nil { panic(err) }
   for _, entry := range block.Entries { msg := new(dns.Msg); if err := msg.Unpack(entry.Msg); err != nil { panic(err) }
    if string(entry.Key[4:]) != "A\\.\\000." || entry.Key[0] != 7 || entry.DomainSet != "go-domain" || entry.MsgStoredTime != 1700000000 || entry.MsgExpirationTime != 1700000060 || entry.CacheExpirationTime != 1700000090 || len(msg.Answer) != 1 { panic("metadata/wire mismatch") }; count++ }
  }; if err := reader.Close(); err != nil { panic(err) }; if count != 1 { panic("wrong entry count") }; fmt.Println("Go verified native v2 export: key/wire/timestamps/domain_set/footer PASS")
 }
}
