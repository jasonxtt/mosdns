// Offline oracle: miekg/dns v1.1.72 Msg.Truncate, no network I/O.
package main

import (
	"bufio"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"

	"github.com/miekg/dns"
)

type input struct {
	Name string `json:"name"`
	Wire string `json:"wire"`
	Size int    `json:"size"`
}

func main() {
	scanner := bufio.NewScanner(os.Stdin)
	scanner.Buffer(make([]byte, 4096), 2*1024*1024)
	encoder := json.NewEncoder(os.Stdout)
	for scanner.Scan() {
		var in input
		if err := json.Unmarshal(scanner.Bytes(), &in); err != nil {
			panic(err)
		}
		wire, err := hex.DecodeString(in.Wire)
		if err != nil {
			panic(err)
		}
		var message dns.Msg
		if err := message.Unpack(wire); err != nil {
			panic(err)
		}
		message.Truncate(in.Size)
		result, err := message.Pack()
		if err != nil {
			panic(err)
		}
		if err := encoder.Encode(map[string]any{"name": in.Name, "wire": hex.EncodeToString(result), "size": len(result), "tc": message.Truncated,
			"answer": len(message.Answer), "authority": len(message.Ns), "additional": len(message.Extra)}); err != nil {
			panic(err)
		}
	}
	if err := scanner.Err(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
