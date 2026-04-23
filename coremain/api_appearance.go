package coremain

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strings"

	"github.com/IrineSistiana/mosdns/v5/mlog"
	"github.com/go-chi/chi/v5"
	"go.uber.org/zap"
)

const (
	appearanceSettingsFilename = "appearance_settings.json"
	panelBackgroundImageRel    = "webinfo/panel_background.bin"
	panelBackgroundMaxUpload   = 20 * 1024 * 1024
)

type panelBackgroundSettings struct {
	Mode    string  `json:"mode"`
	URL     string  `json:"url,omitempty"`
	Opacity float64 `json:"opacity"`
	Blur    int     `json:"blur"`
}

type panelBackgroundResponse struct {
	Mode     string  `json:"mode"`
	URL      string  `json:"url,omitempty"`
	ImageURL string  `json:"image_url,omitempty"`
	Opacity  float64 `json:"opacity"`
	Blur     int     `json:"blur"`
}

func RegisterAppearanceAPI(router *chi.Mux) {
	router.Route("/api/v1/appearance", func(r chi.Router) {
		r.Get("/panel-background", handleGetPanelBackground)
		r.Post("/panel-background", handleSetPanelBackground)
		r.Post("/panel-background/upload", handleUploadPanelBackground)
		r.Get("/panel-background/image", handleGetPanelBackgroundImage)
	})
}

func handleGetPanelBackground(w http.ResponseWriter, r *http.Request) {
	settings, err := loadPanelBackgroundSettings()
	if err != nil {
		mlog.L().Warn("load panel background settings failed, fallback to defaults", zap.Error(err))
		settings = defaultPanelBackgroundSettings()
	}
	writeJSON(w, http.StatusOK, panelBackgroundToResponse(settings))
}

func handleSetPanelBackground(w http.ResponseWriter, r *http.Request) {
	var payload panelBackgroundSettings
	if err := json.NewDecoder(r.Body).Decode(&payload); err != nil {
		writeError(w, http.StatusBadRequest, fmt.Errorf("invalid request body: %w", err))
		return
	}

	settings := normalizePanelBackgroundSettings(payload)
	if settings.Mode == "url" && strings.TrimSpace(settings.URL) == "" {
		writeError(w, http.StatusBadRequest, errors.New("url 不能为空"))
		return
	}
	if settings.Mode == "upload" {
		if _, err := os.Stat(panelBackgroundImagePath()); err != nil {
			writeError(w, http.StatusBadRequest, errors.New("请先上传本地图片，再应用本地背景"))
			return
		}
		settings.URL = ""
	}
	if settings.Mode != "url" {
		settings.URL = ""
	}

	if err := savePanelBackgroundSettings(settings); err != nil {
		writeError(w, http.StatusInternalServerError, fmt.Errorf("save panel background settings failed: %w", err))
		return
	}

	writeJSON(w, http.StatusOK, panelBackgroundToResponse(settings))
}

func handleUploadPanelBackground(w http.ResponseWriter, r *http.Request) {
	r.Body = http.MaxBytesReader(w, r.Body, panelBackgroundMaxUpload+1024)
	if err := r.ParseMultipartForm(panelBackgroundMaxUpload + 1024); err != nil {
		writeError(w, http.StatusBadRequest, fmt.Errorf("解析上传内容失败: %w", err))
		return
	}

	file, header, err := r.FormFile("file")
	if err != nil {
		writeError(w, http.StatusBadRequest, fmt.Errorf("缺少上传文件: %w", err))
		return
	}
	defer file.Close()

	if header != nil && header.Size > panelBackgroundMaxUpload {
		writeError(w, http.StatusBadRequest, errors.New("图片大小不能超过 20MB"))
		return
	}

	data, err := io.ReadAll(io.LimitReader(file, panelBackgroundMaxUpload+1))
	if err != nil {
		writeError(w, http.StatusBadRequest, fmt.Errorf("读取上传文件失败: %w", err))
		return
	}
	if len(data) == 0 {
		writeError(w, http.StatusBadRequest, errors.New("上传文件为空"))
		return
	}
	if len(data) > panelBackgroundMaxUpload {
		writeError(w, http.StatusBadRequest, errors.New("图片大小不能超过 20MB"))
		return
	}

	contentType := http.DetectContentType(data)
	if !strings.HasPrefix(contentType, "image/") {
		writeError(w, http.StatusBadRequest, fmt.Errorf("仅支持图片文件，当前类型: %s", contentType))
		return
	}

	if err := writeManagedFile(panelBackgroundImagePath(), data, nil, nil, nil); err != nil {
		writeError(w, http.StatusInternalServerError, fmt.Errorf("保存上传图片失败: %w", err))
		return
	}

	imageURL := resolveUploadedPanelBackgroundURL()
	writeJSON(w, http.StatusOK, map[string]any{
		"message":   "上传成功",
		"image_url": imageURL,
		"size":      len(data),
	})
}

func handleGetPanelBackgroundImage(w http.ResponseWriter, r *http.Request) {
	imagePath := panelBackgroundImagePath()
	data, err := os.ReadFile(imagePath)
	if err != nil {
		if os.IsNotExist(err) {
			writeError(w, http.StatusNotFound, errors.New("未找到已上传的背景图片"))
			return
		}
		writeError(w, http.StatusInternalServerError, fmt.Errorf("读取背景图片失败: %w", err))
		return
	}
	if len(data) == 0 {
		writeError(w, http.StatusNotFound, errors.New("背景图片为空"))
		return
	}

	info, err := os.Stat(imagePath)
	if err != nil {
		writeError(w, http.StatusInternalServerError, fmt.Errorf("读取背景图片信息失败: %w", err))
		return
	}

	contentType := http.DetectContentType(data)
	if !strings.HasPrefix(contentType, "image/") {
		contentType = "application/octet-stream"
	}
	w.Header().Set("Content-Type", contentType)
	w.Header().Set("Cache-Control", "public, max-age=300")
	http.ServeContent(w, r, filepath.Base(imagePath), info.ModTime(), bytes.NewReader(data))
}

func defaultPanelBackgroundSettings() panelBackgroundSettings {
	return panelBackgroundSettings{
		Mode:    "none",
		URL:     "",
		Opacity: 0.9,
		Blur:    10,
	}
}

func normalizePanelBackgroundSettings(raw panelBackgroundSettings) panelBackgroundSettings {
	out := defaultPanelBackgroundSettings()
	switch strings.ToLower(strings.TrimSpace(raw.Mode)) {
	case "url":
		out.Mode = "url"
	case "upload":
		out.Mode = "upload"
	default:
		out.Mode = "none"
	}
	out.URL = strings.TrimSpace(raw.URL)
	if raw.Opacity >= 0 && raw.Opacity <= 1 {
		out.Opacity = raw.Opacity
	} else if raw.Opacity < 0 {
		out.Opacity = 0
	} else if raw.Opacity > 1 {
		out.Opacity = 1
	}
	if raw.Blur >= 0 && raw.Blur <= 40 {
		out.Blur = raw.Blur
	} else if raw.Blur < 0 {
		out.Blur = 0
	} else if raw.Blur > 40 {
		out.Blur = 40
	}
	return out
}

func panelBackgroundToResponse(settings panelBackgroundSettings) panelBackgroundResponse {
	resp := panelBackgroundResponse{
		Mode:    settings.Mode,
		URL:     settings.URL,
		Opacity: settings.Opacity,
		Blur:    settings.Blur,
	}
	switch settings.Mode {
	case "url":
		resp.ImageURL = settings.URL
	case "upload":
		resp.ImageURL = resolveUploadedPanelBackgroundURL()
	}
	return resp
}

func panelBackgroundSettingsPath() string {
	base := MainConfigBaseDir
	if strings.TrimSpace(base) == "" {
		base = "."
	}
	return filepath.Join(base, appearanceSettingsFilename)
}

func panelBackgroundImagePath() string {
	base := MainConfigBaseDir
	if strings.TrimSpace(base) == "" {
		base = "."
	}
	return filepath.Join(base, panelBackgroundImageRel)
}

func resolveUploadedPanelBackgroundURL() string {
	info, err := os.Stat(panelBackgroundImagePath())
	if err != nil {
		return ""
	}
	return fmt.Sprintf("/api/v1/appearance/panel-background/image?v=%d", info.ModTime().UnixNano())
}

func loadPanelBackgroundSettings() (panelBackgroundSettings, error) {
	path := panelBackgroundSettingsPath()
	raw, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return defaultPanelBackgroundSettings(), nil
		}
		return defaultPanelBackgroundSettings(), err
	}
	var parsed panelBackgroundSettings
	if err := json.Unmarshal(raw, &parsed); err != nil {
		return defaultPanelBackgroundSettings(), err
	}
	return normalizePanelBackgroundSettings(parsed), nil
}

func savePanelBackgroundSettings(settings panelBackgroundSettings) error {
	normalized := normalizePanelBackgroundSettings(settings)
	data, err := json.MarshalIndent(normalized, "", "  ")
	if err != nil {
		return err
	}
	return writeManagedFile(panelBackgroundSettingsPath(), data, func(raw []byte) error {
		var parsed panelBackgroundSettings
		if err := json.Unmarshal(raw, &parsed); err != nil {
			return err
		}
		_ = normalizePanelBackgroundSettings(parsed)
		return nil
	}, nil, nil)
}
