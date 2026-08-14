package ip_set

import matcheradapter "github.com/IrineSistiana/mosdns/v5/plugin/data_provider/matcher_adapter"

func buildRustIPMatcher(prefixes []string) (RustMatcher, error) {
	snapshot, err := matcheradapter.BuildIPSnapshot(prefixes)
	if err != nil || snapshot == nil {
		return snapshot, err
	}
	return snapshot, nil
}

const rustIPMatcherBackendEnv = matcheradapter.BackendEnv
