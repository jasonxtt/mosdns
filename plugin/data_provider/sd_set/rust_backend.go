package sd_set

import (
	"github.com/IrineSistiana/mosdns/v5/pkg/matcher/domain"
	"github.com/IrineSistiana/mosdns/v5/plugin/data_provider/matcher_adapter"
)

var rustDomainSnapshotBuilder = matcher_adapter.BuildDomainSnapshot

type sdGeneration struct {
	goMatcher   *domain.MixMatcher[struct{}]
	rustMatcher matcher_adapter.DomainSnapshot
}

func (p *SdSet) publishGeneration(next *sdGeneration) {
	p.generationMu.Lock()
	old := p.generation
	p.generation = next
	p.matcher.Store(next.goMatcher)
	p.generationMu.Unlock()

	if old != nil && old.rustMatcher != nil {
		_ = old.rustMatcher.Close()
	}
}

func (p *SdSet) disableRustGeneration(generation *sdGeneration) {
	p.generationMu.Lock()
	var retired matcher_adapter.DomainSnapshot
	if p.generation == generation && generation.rustMatcher != nil {
		retired = generation.rustMatcher
		generation.rustMatcher = nil
	}
	p.generationMu.Unlock()

	if retired != nil {
		_ = retired.Close()
	}
}

func (p *SdSet) matchGeneration(domainStr string) (bool, bool) {
	p.generationMu.RLock()
	generation := p.generation
	if generation == nil {
		p.generationMu.RUnlock()
		return false, false
	}

	if rustMatcher := generation.rustMatcher; rustMatcher != nil {
		matched, err := rustMatcher.Match(domainStr)
		p.generationMu.RUnlock()
		if err == nil {
			return matched, true
		}
		p.disableRustGeneration(generation)
		_, matched = generation.goMatcher.Match(domainStr)
		return matched, true
	}
	_, matched := generation.goMatcher.Match(domainStr)
	p.generationMu.RUnlock()
	return matched, true
}

func newSdGeneration(goMatcher *domain.MixMatcher[struct{}], rustMatcher matcher_adapter.DomainSnapshot) *sdGeneration {
	return &sdGeneration{
		goMatcher:   goMatcher,
		rustMatcher: rustMatcher,
	}
}
