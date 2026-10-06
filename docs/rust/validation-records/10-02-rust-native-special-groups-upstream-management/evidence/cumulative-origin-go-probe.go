package main

import (
 "compress/gzip"
 "encoding/binary"
 "encoding/json"
 "fmt"
 "io"
 "os"
 cache "github.com/IrineSistiana/mosdns/v5/plugin/executable/cache"
 "google.golang.org/protobuf/proto"
)

func main() {
 records := []map[string]any{}
 for _, path := range os.Args[1:] {
  f, err := os.Open(path); if err != nil { panic(err) }
  reader, err := gzip.NewReader(f); if err != nil { panic(err) }
  if reader.Name != "mosdns_cache_v2" { panic("changed gzip name") }
  entries, unknown := 0,0
  for {
   var length [8]byte
   _,err = io.ReadFull(reader,length[:]); if err==io.EOF { break }; if err!=nil { panic(err) }
   size := binary.BigEndian.Uint64(length[:]); if size>1024*1024 { panic("block limit") }
   data := make([]byte,size); if _,err=io.ReadFull(reader,data); err!=nil { panic(err) }
   block := &cache.CacheDumpBlock{}; if err=proto.Unmarshal(data,block); err!=nil { panic(err) }
   for _,entry := range block.Entries {
    if len(entry.Key)==0 || len(entry.Msg)==0 || entry.MsgStoredTime<=0 { panic("legacy fields missing") }
    if len(entry.ProtoReflect().GetUnknown())>0 { unknown++ }
    entries++
   }
  }
  reader.Close();f.Close()
  if entries==0 || unknown!=entries { panic("native extension missing") }
  records=append(records,map[string]any{"path":path,"entries":entries,"unknown_native_extension_entries":unknown,"legacy_reader":"PASS"})
 }
 bytes,err:=json.MarshalIndent(records,"","  ");if err!=nil {panic(err)};fmt.Println(string(bytes))
}
