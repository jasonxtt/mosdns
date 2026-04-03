package coremain

import (
	"encoding/json"
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"

	"github.com/go-chi/chi/v5"
)

const (
	specialGroupsFilename = "special_upstream_groups.json"
	specialSlotMin        = 50
	specialSlotMax        = 59
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

	slot := payload.Slot
	if slot == 0 {
		slot = firstFreeSpecialSlot(specialGroups)
		if slot == 0 {
			http.Error(w, `{"error":"no free special upstream slots (50-59)"}`, http.StatusConflict)
			return
		}
	}
	if slot < specialSlotMin || slot > specialSlotMax {
		http.Error(w, `{"error":"slot must be between 50 and 59"}`, http.StatusBadRequest)
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
	}
	sort.Slice(specialGroups, func(i, j int) bool { return specialGroups[i].Slot < specialGroups[j].Slot })

	if err := saveSpecialGroupsLocked(); err != nil {
		http.Error(w, `{"error":"failed to save special groups"}`, http.StatusInternalServerError)
		return
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
	if slot < specialSlotMin || slot > specialSlotMax {
		http.Error(w, `{"error":"slot must be between 50 and 59"}`, http.StatusBadRequest)
		return
	}

	specialGroupsLock.Lock()
	defer specialGroupsLock.Unlock()

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
		http.Error(w, `{"error":"failed to save special groups"}`, http.StatusInternalServerError)
		return
	}

	_ = clearSpecialGroupArtifacts(slot)
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

	dir := MainConfigBaseDir
	if dir == "" {
		dir = "."
	}
	path := filepath.Join(dir, specialGroupsFilename)
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			specialGroups = make([]SpecialGroup, 0)
			return nil
		}
		return err
	}
	if len(data) == 0 {
		specialGroups = make([]SpecialGroup, 0)
		return nil
	}

	var groups []SpecialGroup
	if err := json.Unmarshal(data, &groups); err != nil {
		return err
	}

	filtered := make([]SpecialGroup, 0, len(groups))
	seen := make(map[int]struct{})
	for _, g := range groups {
		g.Name = strings.TrimSpace(g.Name)
		if g.Slot < specialSlotMin || g.Slot > specialSlotMax || g.Name == "" {
			continue
		}
		if _, ok := seen[g.Slot]; ok {
			continue
		}
		seen[g.Slot] = struct{}{}
		filtered = append(filtered, g)
	}
	sort.Slice(filtered, func(i, j int) bool { return filtered[i].Slot < filtered[j].Slot })
	specialGroups = filtered
	return nil
}

func saveSpecialGroupsLocked() error {
	dir := MainConfigBaseDir
	if dir == "" {
		dir = "."
	}

	path := filepath.Join(dir, specialGroupsFilename)
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
			if g.Slot < specialSlotMin || g.Slot > specialSlotMax {
				return fmt.Errorf("slot must be between %d and %d", specialSlotMin, specialSlotMax)
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

func firstFreeSpecialSlot(groups []SpecialGroup) int {
	used := make(map[int]struct{}, len(groups))
	for _, g := range groups {
		used[g.Slot] = struct{}{}
	}
	for slot := specialSlotMin; slot <= specialSlotMax; slot++ {
		if _, ok := used[slot]; !ok {
			return slot
		}
	}
	return 0
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

func clearSpecialGroupArtifacts(slot int) error {
	dir := MainConfigBaseDir
	if dir == "" {
		dir = "."
	}

	if err := loadUpstreamOverrides(); err == nil {
		upstreamOverridesLock.Lock()
		delete(upstreamOverrides, specialUpstreamPluginTag(slot))
		saveErr := saveUpstreamOverrides()
		upstreamOverridesLock.Unlock()
		if saveErr != nil {
			return saveErr
		}
	}

	localConfigPath := filepath.Join(dir, "srs", fmt.Sprintf("special_%d.json", slot))
	if err := os.Remove(localConfigPath); err != nil && !os.IsNotExist(err) {
		return err
	}

	manualRulePath := filepath.Join(dir, specialManualRulePath(slot))
	if err := os.Remove(manualRulePath); err != nil && !os.IsNotExist(err) {
		return err
	}
	return nil
}
