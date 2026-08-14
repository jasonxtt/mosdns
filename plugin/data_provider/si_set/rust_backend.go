package si_set

import (
	"net/netip"

	"github.com/IrineSistiana/mosdns/v5/pkg/matcher/netlist"
	"github.com/IrineSistiana/mosdns/v5/plugin/data_provider/matcher_adapter"
)

var rustIPSnapshotBuilder = matcher_adapter.BuildIPSnapshot

type siGeneration struct {
	goMatcher   *netlist.List
	rustMatcher matcher_adapter.IPSnapshot
}

func (p *SiSet) publishGeneration(next *siGeneration) {
	p.generationMu.Lock()
	old := p.generation
	p.generation = next
	p.matcher.Store(next.goMatcher)
	p.generationMu.Unlock()

	if old != nil && old.rustMatcher != nil {
		_ = old.rustMatcher.Close()
	}
}

func (p *SiSet) disableRustGeneration(generation *siGeneration) {
	p.generationMu.Lock()
	var retired matcher_adapter.IPSnapshot
	if p.generation == generation && generation.rustMatcher != nil {
		retired = generation.rustMatcher
		generation.rustMatcher = nil
	}
	p.generationMu.Unlock()

	if retired != nil {
		_ = retired.Close()
	}
}

func (p *SiSet) matchGeneration(addr netip.Addr) (bool, bool) {
	p.generationMu.RLock()
	generation := p.generation
	if generation == nil {
		p.generationMu.RUnlock()
		return false, false
	}

	if rustMatcher := generation.rustMatcher; rustMatcher != nil {
		matched, err := rustMatcher.Match(addr.String())
		p.generationMu.RUnlock()
		if err == nil {
			return matched, true
		}
		p.disableRustGeneration(generation)
		return generation.goMatcher.Match(addr), true
	}
	matched := generation.goMatcher.Match(addr)
	p.generationMu.RUnlock()
	return matched, true
}

func newSiGeneration(goMatcher *netlist.List, rustMatcher matcher_adapter.IPSnapshot) *siGeneration {
	return &siGeneration{
		goMatcher:   goMatcher,
		rustMatcher: rustMatcher,
	}
}
