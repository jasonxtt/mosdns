package coremain

import (
    "encoding/json"
    "fmt"
    "net/http"
    "os"
    "path/filepath"

	"github.com/IrineSistiana/mosdns/v5/mlog"
	"github.com/go-chi/chi/v5"
	"go.uber.org/zap"
)

// RegisterOverridesAPI registers the global overrides APIs.
func RegisterOverridesAPI(router *chi.Mux) {
    router.Route("/api/v1/overrides", func(r chi.Router) {
        r.Get("/", handleGetOverrides)
        r.Post("/", handleSetOverrides)
    })
}

// PartialOverrides 用于区分字段是否在 JSON 中出现（指针为 nil 表示未提供）。
type PartialOverrides struct {
    Socks5 *string `json:"socks5"`
    ECS    *string `json:"ecs"`
}

func handleGetOverrides(w http.ResponseWriter, r *http.Request) {
    overridesPath := filepath.Join(MainConfigBaseDir, overridesFilename)
    data, err := os.ReadFile(overridesPath)

	var response GlobalOverrides
	if err == nil {
		// File exists, try to parse it.
		if json.Unmarshal(data, &response) != nil {
			mlog.L().Warn("could not parse config_overrides.json on GET, falling back to discovered settings", zap.Error(err))
			response.Socks5 = discoveredSocks5
			response.ECS = discoveredECS
		}
	} else {
		// File does not exist, use the cached discovered settings.
		response.Socks5 = discoveredSocks5
		response.ECS = discoveredECS
	}

	w.Header().Set("Content-Type", "application/json")
	if err := json.NewEncoder(w).Encode(response); err != nil {
		mlog.L().Error("failed to encode overrides response", zap.Error(err))
	}
}

func handleSetOverrides(w http.ResponseWriter, r *http.Request) {
    var patch PartialOverrides
    if err := json.NewDecoder(r.Body).Decode(&patch); err != nil {
        http.Error(w, "Invalid request body: "+err.Error(), http.StatusBadRequest)
        return
    }

    overridesPath := filepath.Join(MainConfigBaseDir, overridesFilename)
    currentOverrides := GlobalOverrides{}

	// Read existing file to preserve other potential settings in the future.
	data, err := os.ReadFile(overridesPath)
	if err == nil {
		// Ignore parsing errors, we'll just overwrite the relevant fields.
		_ = json.Unmarshal(data, &currentOverrides)
	}

    // 仅在请求中提供了对应字段时才更新，避免空值覆盖。
    if patch.Socks5 != nil {
        currentOverrides.Socks5 = *patch.Socks5
    }
    if patch.ECS != nil {
        currentOverrides.ECS = *patch.ECS
    }

	// Write back to the file.
	updatedData, err := json.MarshalIndent(currentOverrides, "", "  ")
	if err != nil {
		http.Error(w, "Failed to marshal settings: "+err.Error(), http.StatusInternalServerError)
		return
	}

	if err := os.WriteFile(overridesPath, updatedData, 0644); err != nil {
		http.Error(w, "Failed to write settings file: "+err.Error(), http.StatusInternalServerError)
		return
	}

	mlog.L().Info("global overrides saved via API",
		zap.String("socks5", currentOverrides.Socks5),
		zap.String("ecs", currentOverrides.ECS))

	w.Header().Set("Content-Type", "application/json")
	fmt.Fprint(w, `{"message": "Global overrides saved. Please restart mosdns to apply changes."}`)
}
