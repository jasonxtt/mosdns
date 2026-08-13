package testutil

import (
	"bufio"
	"bytes"
	"compress/zlib"
	"encoding/binary"

	scdomain "github.com/sagernet/sing/common/domain"
	"github.com/sagernet/sing/common/varbin"
)

// BuildDomainSRS creates the small v3 SRS fixture used by provider contract tests.
func BuildDomainSRS(domains, suffixes, keywords, regexes []string) ([]byte, error) {
	const (
		ruleItemDomain        = uint8(2)
		ruleItemDomainKeyword = uint8(3)
		ruleItemDomainRegex   = uint8(4)
		ruleItemFinal         = uint8(0xff)
	)

	var compressed bytes.Buffer
	zw := zlib.NewWriter(&compressed)
	bw := bufio.NewWriter(zw)
	var count [binary.MaxVarintLen64]byte
	n := binary.PutUvarint(count[:], 1)
	if _, err := bw.Write(count[:n]); err != nil {
		return nil, err
	}
	if err := bw.WriteByte(0); err != nil {
		return nil, err
	}
	if len(domains) > 0 || len(suffixes) > 0 {
		if err := bw.WriteByte(ruleItemDomain); err != nil {
			return nil, err
		}
		if err := scdomain.NewMatcher(domains, suffixes, true).Write(bw); err != nil {
			return nil, err
		}
	}
	if len(keywords) > 0 {
		if err := bw.WriteByte(ruleItemDomainKeyword); err != nil {
			return nil, err
		}
		if err := varbin.Write(bw, binary.BigEndian, keywords); err != nil {
			return nil, err
		}
	}
	if len(regexes) > 0 {
		if err := bw.WriteByte(ruleItemDomainRegex); err != nil {
			return nil, err
		}
		if err := varbin.Write(bw, binary.BigEndian, regexes); err != nil {
			return nil, err
		}
	}
	if err := bw.WriteByte(ruleItemFinal); err != nil {
		return nil, err
	}
	if err := bw.Flush(); err != nil {
		return nil, err
	}
	if err := zw.Close(); err != nil {
		return nil, err
	}
	return append([]byte{'S', 'R', 'S', 3}, compressed.Bytes()...), nil
}
