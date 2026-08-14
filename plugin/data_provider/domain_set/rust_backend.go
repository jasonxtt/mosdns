package domain_set

import matcheradapter "github.com/IrineSistiana/mosdns/v5/plugin/data_provider/matcher_adapter"

func buildRustDomainMatcher(rules []string) (RustMatcher, error) {
	snapshot, err := matcheradapter.BuildDomainSnapshot(rules)
	if err != nil || snapshot == nil {
		return snapshot, err
	}
	return snapshot, nil
}

const rustMatcherBackendEnv = matcheradapter.BackendEnv
