package coremain

import (
	"encoding/json"
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"sync"

	"github.com/go-chi/chi/v5"
	"gopkg.in/yaml.v3"
)

const (
	specialGroupsFilename           = "special_upstream_groups.json"
	specialGroupsConfigRelativePath = "sub_config/special_groups.yaml"
	specialSlotMin                  = 50
	specialGroupRestartDelayMs      = 1500
)

type SpecialGroup struct {
	Slot int    `json:"slot"`
	Name string `json:"name"`
}

type SpecialGroupView struct {
	Slot               int    `json:"slot"`
	Name               string `json:"name"`
	Key                string `json:"key"`
	UpstreamPluginTag  string `json:"upstream_plugin_tag"`
	DiversionPluginTag string `json:"diversion_plugin_tag"`
	ManualPluginTag    string `json:"manual_plugin_tag"`
	LocalConfig        string `json:"local_config"`
	ManualRulePath     string `json:"manual_rule_path"`
}

var (
	specialGroupsLock sync.RWMutex
	specialGroups     []SpecialGroup
)

func RegisterSpecialGroupsAPI(router *chi.Mux) {
	router.Route("/api/v1/special-groups", func(r chi.Router) {
		r.Get("/", handleGetSpecialGroups)
		r.Post("/", handleSaveSpecialGroup)
		r.Delete("/{slot}", handleDeleteSpecialGroup)
	})
}

func handleGetSpecialGroups(w http.ResponseWriter, r *http.Request) {
	if err := ensureSpecialGroupsLoaded(); err != nil {
		http.Error(w, `{"error":"failed to load special groups"}`, http.StatusInternalServerError)
		return
	}

	specialGroupsLock.RLock()
	defer specialGroupsLock.RUnlock()

	resp := make([]SpecialGroupView, 0, len(specialGroups))
	for _, g := range specialGroups {
		resp = append(resp, buildSpecialGroupView(g))
	}
	sort.Slice(resp, func(i, j int) bool { return resp[i].Slot < resp[j].Slot })

	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(resp)
}

func handleSaveSpecialGroup(w http.ResponseWriter, r *http.Request) {
	if err := ensureSpecialGroupsLoaded(); err != nil {
		http.Error(w, `{"error":"failed to load special groups"}`, http.StatusInternalServerError)
		return
	}

	var payload struct {
		Slot int    `json:"slot"`
		Name string `json:"name"`
	}
	if err := json.NewDecoder(r.Body).Decode(&payload); err != nil {
		http.Error(w, `{"error":"invalid request body"}`, http.StatusBadRequest)
		return
	}

	payload.Name = strings.TrimSpace(payload.Name)
	if payload.Name == "" {
		http.Error(w, `{"error":"name is required"}`, http.StatusBadRequest)
		return
	}

	specialGroupsLock.Lock()
	defer specialGroupsLock.Unlock()

	oldState := cloneSpecialGroups(specialGroups)
	slot := payload.Slot
	created := false

	if slot == 0 {
		slot = firstFreeSpecialSlot(specialGroups)
	}
	if !isValidSpecialSlot(slot) {
		http.Error(w, fmt.Sprintf(`{"error":"slot must be >= %d"}`, specialSlotMin), http.StatusBadRequest)
		return
	}

	for _, g := range specialGroups {
		if g.Slot != slot && strings.EqualFold(g.Name, payload.Name) {
			http.Error(w, `{"error":"name already exists"}`, http.StatusConflict)
			return
		}
	}

	updated := false
	for i := range specialGroups {
		if specialGroups[i].Slot == slot {
			specialGroups[i].Name = payload.Name
			updated = true
			break
		}
	}
	if !updated {
		specialGroups = append(specialGroups, SpecialGroup{Slot: slot, Name: payload.Name})
		created = true
	}
	sort.Slice(specialGroups, func(i, j int) bool { return specialGroups[i].Slot < specialGroups[j].Slot })

	if err := saveSpecialGroupsLocked(); err != nil {
		specialGroups = oldState
		http.Error(w, `{"error":"failed to save special groups"}`, http.StatusInternalServerError)
		return
	}

	if created {
		if err := syncSpecialGroupsConfigLocked(); err != nil {
			rollbackSpecialGroupsLocked(oldState)
			http.Error(w, `{"error":"failed to update special groups config"}`, http.StatusInternalServerError)
			return
		}
		_ = scheduleSelfRestart(GetCurrentMosdns(), specialGroupRestartDelayMs)
	}

	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(buildSpecialGroupView(SpecialGroup{Slot: slot, Name: payload.Name}))
}

func handleDeleteSpecialGroup(w http.ResponseWriter, r *http.Request) {
	if err := ensureSpecialGroupsLoaded(); err != nil {
		http.Error(w, `{"error":"failed to load special groups"}`, http.StatusInternalServerError)
		return
	}

	var slot int
	if _, err := fmt.Sscanf(chi.URLParam(r, "slot"), "%d", &slot); err != nil {
		http.Error(w, `{"error":"invalid slot"}`, http.StatusBadRequest)
		return
	}
	if !isValidSpecialSlot(slot) {
		http.Error(w, fmt.Sprintf(`{"error":"slot must be >= %d"}`, specialSlotMin), http.StatusBadRequest)
		return
	}

	specialGroupsLock.Lock()
	defer specialGroupsLock.Unlock()

	oldState := cloneSpecialGroups(specialGroups)
	next := make([]SpecialGroup, 0, len(specialGroups))
	found := false
	for _, g := range specialGroups {
		if g.Slot == slot {
			found = true
			continue
		}
		next = append(next, g)
	}
	if !found {
		http.Error(w, `{"error":"special group not found"}`, http.StatusNotFound)
		return
	}

	specialGroups = next
	if err := saveSpecialGroupsLocked(); err != nil {
		specialGroups = oldState
		http.Error(w, `{"error":"failed to save special groups"}`, http.StatusInternalServerError)
		return
	}
	if err := syncSpecialGroupsConfigLocked(); err != nil {
		rollbackSpecialGroupsLocked(oldState)
		http.Error(w, `{"error":"failed to update special groups config"}`, http.StatusInternalServerError)
		return
	}

	_ = clearSpecialGroupArtifacts(slot)
	_ = scheduleSelfRestart(GetCurrentMosdns(), specialGroupRestartDelayMs)
	w.WriteHeader(http.StatusNoContent)
}

func ensureSpecialGroupsLoaded() error {
	specialGroupsLock.RLock()
	loaded := specialGroups != nil
	specialGroupsLock.RUnlock()
	if loaded {
		return nil
	}
	return loadSpecialGroups()
}

func loadSpecialGroups() error {
	specialGroupsLock.Lock()
	defer specialGroupsLock.Unlock()

	groups, err := loadSpecialGroupsFromDir(mainConfigDir())
	if err != nil {
		return err
	}
	specialGroups = groups
	return nil
}

func loadSpecialGroupsFromDir(dir string) ([]SpecialGroup, error) {
	if dir == "" {
		dir = "."
	}

	path := managedStateFilePathInDir(dir, specialGroupsFilename)
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return make([]SpecialGroup, 0), nil
		}
		return nil, err
	}
	if len(data) == 0 {
		return make([]SpecialGroup, 0), nil
	}

	var groups []SpecialGroup
	if err := json.Unmarshal(data, &groups); err != nil {
		return nil, err
	}
	return normalizeSpecialGroups(groups), nil
}

func normalizeSpecialGroups(groups []SpecialGroup) []SpecialGroup {
	filtered := make([]SpecialGroup, 0, len(groups))
	seen := make(map[int]struct{}, len(groups))
	for _, g := range groups {
		g.Name = strings.TrimSpace(g.Name)
		if !isValidSpecialSlot(g.Slot) || g.Name == "" {
			continue
		}
		if _, ok := seen[g.Slot]; ok {
			continue
		}
		seen[g.Slot] = struct{}{}
		filtered = append(filtered, g)
	}
	sort.Slice(filtered, func(i, j int) bool { return filtered[i].Slot < filtered[j].Slot })
	return filtered
}

func saveSpecialGroupsLocked() error {
	path := managedStateFilePathInDir(mainConfigDir(), specialGroupsFilename)
	data, err := json.MarshalIndent(specialGroups, "", "  ")
	if err != nil {
		return err
	}
	return writeManagedFile(path, data, func(raw []byte) error {
		var groups []SpecialGroup
		if err := json.Unmarshal(raw, &groups); err != nil {
			return err
		}
		seen := make(map[int]struct{}, len(groups))
		for _, g := range groups {
			if !isValidSpecialSlot(g.Slot) {
				return fmt.Errorf("slot must be >= %d", specialSlotMin)
			}
			name := strings.TrimSpace(g.Name)
			if name == "" {
				return fmt.Errorf("name is required")
			}
			if _, ok := seen[g.Slot]; ok {
				return fmt.Errorf("duplicate slot %d", g.Slot)
			}
			seen[g.Slot] = struct{}{}
		}
		return nil
	}, nil, nil)
}

func cloneSpecialGroups(groups []SpecialGroup) []SpecialGroup {
	cloned := make([]SpecialGroup, len(groups))
	copy(cloned, groups)
	return cloned
}

func rollbackSpecialGroupsLocked(oldState []SpecialGroup) {
	specialGroups = cloneSpecialGroups(oldState)
	_ = saveSpecialGroupsLocked()
	_ = syncSpecialGroupsConfigLocked()
}

func firstFreeSpecialSlot(groups []SpecialGroup) int {
	used := make(map[int]struct{}, len(groups))
	for _, g := range groups {
		used[g.Slot] = struct{}{}
	}
	for slot := specialSlotMin; ; slot++ {
		if _, ok := used[slot]; !ok {
			return slot
		}
	}
}

func buildSpecialGroupView(g SpecialGroup) SpecialGroupView {
	return SpecialGroupView{
		Slot:               g.Slot,
		Name:               g.Name,
		Key:                specialGroupKey(g.Slot),
		UpstreamPluginTag:  specialUpstreamPluginTag(g.Slot),
		DiversionPluginTag: specialDiversionPluginTag(g.Slot),
		ManualPluginTag:    specialManualPluginTag(g.Slot),
		LocalConfig:        fmt.Sprintf("srs/special_%d.json", g.Slot),
		ManualRulePath:     specialManualRulePath(g.Slot),
	}
}

func specialGroupKey(slot int) string {
	return fmt.Sprintf("special_%d", slot)
}

func specialUpstreamPluginTag(slot int) string {
	return fmt.Sprintf("special_upstream_%d", slot)
}

func specialDiversionPluginTag(slot int) string {
	return fmt.Sprintf("special_route_%d", slot)
}

func specialManualPluginTag(slot int) string {
	return fmt.Sprintf("special_manual_%d", slot)
}

func specialManualRulePath(slot int) string {
	return filepath.Join("rule", fmt.Sprintf("special_%d.txt", slot))
}

func isValidSpecialSlot(slot int) bool {
	return slot >= specialSlotMin
}

func mainConfigDir() string {
	dir := MainConfigBaseDir
	if dir == "" {
		dir = "."
	}
	return dir
}

func syncSpecialGroupsConfigLocked() error {
	return writeSpecialGroupsConfig(mainConfigDir(), specialGroups)
}

func SyncSpecialGroupsConfig(baseDir string) error {
	groups, err := loadSpecialGroupsFromDir(baseDir)
	if err != nil {
		return err
	}
	if err := cleanupOrphanSpecialCacheDumps(baseDir, groups); err != nil {
		return err
	}
	return writeSpecialGroupsConfig(baseDir, groups)
}

func writeSpecialGroupsConfig(baseDir string, groups []SpecialGroup) error {
	if baseDir == "" {
		baseDir = "."
	}
	path := filepath.Join(baseDir, specialGroupsConfigRelativePath)
	data := renderSpecialGroupsConfig(groups)
	return writeManagedFile(path, data, validateSpecialGroupsConfig, nil, nil)
}

func validateSpecialGroupsConfig(raw []byte) error {
	var cfg Config
	return yaml.Unmarshal(raw, &cfg)
}

func renderSpecialGroupsConfig(groups []SpecialGroup) []byte {
	var b strings.Builder

	b.WriteString("plugins:\n")
	b.WriteString("# 由 special_groups API 自动生成。不要手动编辑。\n")
	for _, g := range groups {
		slot := g.Slot
		b.WriteString(fmt.Sprintf("  - tag: special_route_%d\n", slot))
		b.WriteString("    type: sd_set_light\n")
		b.WriteString("    args:\n")
		b.WriteString("      socks5: \"\"\n")
		b.WriteString(fmt.Sprintf("      local_config: \"srs/special_%d.json\"\n\n", slot))

		b.WriteString(fmt.Sprintf("  - tag: special_manual_%d\n", slot))
		b.WriteString("    type: domain_set_light\n")
		b.WriteString("    args:\n")
		b.WriteString("      files:\n")
		b.WriteString(fmt.Sprintf("        - \"rule/special_%d.txt\"\n\n", slot))

		b.WriteString(fmt.Sprintf("  - tag: cache_special_%d\n", slot))
		b.WriteString("    type: cache\n")
		b.WriteString("    args:\n")
		b.WriteString("      size: 20000000\n")
		b.WriteString("      lazy_cache_ttl: 259200000\n")
		b.WriteString(fmt.Sprintf("      dump_file: cache/cache_special_%d.dump\n", slot))
		b.WriteString("      dump_interval: 36000\n\n")

		b.WriteString(fmt.Sprintf("  - tag: special_upstream_%d\n", slot))
		b.WriteString("    type: aliapi\n")
		b.WriteString("    args:\n")
		b.WriteString("      concurrent: 2\n")
		b.WriteString("      upstreams:\n")
		b.WriteString("        - addr: \"223.5.5.5\"\n\n")

		b.WriteString(fmt.Sprintf("  - tag: sequence_special_%d\n", slot))
		b.WriteString("    type: sequence\n")
		b.WriteString("    args:\n")
		b.WriteString("      - matches: switch4 'A'\n")
		b.WriteString(fmt.Sprintf("        exec: $cache_special_%d\n", slot))
		b.WriteString(fmt.Sprintf("      - exec: flow_setter group=special_%d sequence=sequence_special_%d upstream=special_upstream_%d\n", slot, slot, slot))
		b.WriteString(fmt.Sprintf("      - exec: $special_upstream_%d\n", slot))
		b.WriteString("      - exec: cname_remover\n\n")
	}

	b.WriteString("  - tag: special_upstream_matcher\n")
	b.WriteString("    type: domain_mapper\n")
	b.WriteString("    args:\n")
	b.WriteString("      default_mark: 0\n")
	b.WriteString("      default_tag: \"\"\n")
	if len(groups) == 0 {
		b.WriteString("      rules: []\n\n")
	} else {
		b.WriteString("      rules:\n")
		for _, g := range groups {
			slot := g.Slot
			b.WriteString(fmt.Sprintf("        - tag: special_route_%d\n", slot))
			b.WriteString(fmt.Sprintf("          ctx_mark: %d\n", slot))
			b.WriteString(fmt.Sprintf("          output_tag: \"特殊上游%d\"\n", slot))
			b.WriteString(fmt.Sprintf("        - tag: special_manual_%d\n", slot))
			b.WriteString(fmt.Sprintf("          ctx_mark: %d\n", slot))
			b.WriteString(fmt.Sprintf("          output_tag: \"特殊上游%d\"\n", slot))
		}
		b.WriteString("\n")
	}

	b.WriteString("  - tag: sequence_special_all\n")
	b.WriteString("    type: sequence\n")
	if len(groups) == 0 {
		b.WriteString("    args: []\n")
	} else {
		b.WriteString("    args:\n")
		for _, g := range groups {
			slot := g.Slot
			b.WriteString(fmt.Sprintf("      - matches: mark %d\n", slot))
			b.WriteString(fmt.Sprintf("        exec: $sequence_special_%d\n", slot))
			b.WriteString(fmt.Sprintf("      - matches: mark %d\n", slot))
			b.WriteString("        exec: exit\n")
		}
	}

	return []byte(b.String())
}

func clearSpecialGroupArtifacts(slot int) error {
	dir := mainConfigDir()

	if err := loadUpstreamOverrides(); err == nil {
		upstreamOverridesLock.Lock()
		delete(upstreamOverrides, specialUpstreamPluginTag(slot))
		saveErr := saveUpstreamOverrides()
		upstreamOverridesLock.Unlock()
		if saveErr != nil {
			return saveErr
		}
	}

	paths := []string{
		filepath.Join(dir, "srs", fmt.Sprintf("special_%d.json", slot)),
		filepath.Join(dir, specialManualRulePath(slot)),
		filepath.Join(dir, "cache", fmt.Sprintf("cache_special_%d.dump", slot)),
	}
	for _, p := range paths {
		if err := os.Remove(p); err != nil && !os.IsNotExist(err) {
			return err
		}
	}
	return nil
}

func cleanupOrphanSpecialCacheDumps(baseDir string, groups []SpecialGroup) error {
	if baseDir == "" {
		baseDir = "."
	}

	activeSlots := make(map[int]struct{}, len(groups))
	for _, g := range groups {
		activeSlots[g.Slot] = struct{}{}
	}

	pattern := filepath.Join(baseDir, "cache", "cache_special_*.dump")
	matches, err := filepath.Glob(pattern)
	if err != nil {
		return err
	}

	for _, path := range matches {
		name := filepath.Base(path)
		slotText := strings.TrimSuffix(strings.TrimPrefix(name, "cache_special_"), ".dump")
		slot, err := strconv.Atoi(slotText)
		if err != nil || !isValidSpecialSlot(slot) {
			continue
		}
		if _, ok := activeSlots[slot]; ok {
			continue
		}
		if err := os.Remove(path); err != nil && !os.IsNotExist(err) {
			return err
		}
	}

	return nil
}
