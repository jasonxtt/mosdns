//go:build !linux

package udp_server

import "time"


func getBootTimeNano() uint64 {
	return uint64(time.Now().UnixNano())
}
