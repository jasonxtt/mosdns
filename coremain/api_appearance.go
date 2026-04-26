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
	"regexp"
	"sort"
	"strings"
	"time"

	"github.com/IrineSistiana/mosdns/v5/mlog"
	"github.com/go-chi/chi/v5"
	"go.uber.org/zap"
)

const (
	appearanceSettingsFilename = "appearance_settings.json"
	appearanceTextSettingsFile = "appearance_text_settings.json"
	panelBackgroundImageRel    = "webinfo/panel_background.bin"
	panelBackgroundHistoryRel  = "webinfo/panel_background_history"
	panelBackgroundMaxUpload   = 20 * 1024 * 1024
)

var hexColorRegexp = regexp.MustCompile(`^#?([0-9a-fA-F]{6})$`)
var panelBackgroundUploadIDRegexp = regexp.MustCompile(`^[a-zA-Z0-9_-]{6,64}$`)

type panelBackgroundSettings struct {
	Mode       string  `json:"mode"`
	URL        string  `json:"url,omitempty"`
	Color      string  `json:"color,omitempty"` // backward compatible legacy field
	LightColor string  `json:"light_color,omitempty"`
	DarkColor  string  `json:"dark_color,omitempty"`
	UploadID   string  `json:"upload_id,omitempty"`
	Opacity    float64 `json:"opacity"`
	Blur       int     `json:"blur"`
}

type panelBackgroundResponse struct {
	Mode       string  `json:"mode"`
	URL        string  `json:"url,omitempty"`
	Color      string  `json:"color,omitempty"` // backward compatible legacy field
	LightColor string  `json:"light_color,omitempty"`
	DarkColor  string  `json:"dark_color,omitempty"`
	UploadID   string  `json:"upload_id,omitempty"`
	ImageURL   string  `json:"image_url,omitempty"`
	Opacity    float64 `json:"opacity"`
	Blur       int     `json:"blur"`
}

type panelBackgroundHistoryItem struct {
	ID         string `json:"id"`
	ImageURL   string `json:"image_url"`
	Size       int64  `json:"size"`
	ModifiedAt string `json:"modified_at"`
}

type textColorSetting struct {
	Mode  string `json:"mode"`
	Color string `json:"color,omitempty"`
}

type textColorSettings struct {
	Light textColorSetting `json:"light"`
	Dark  textColorSetting `json:"dark"`
}

func RegisterAppearanceAPI(router *chi.Mux) {
	router.Route("/api/v1/appearance", func(r chi.Router) {
		r.Get("/panel-background", handleGetPanelBackground)
		r.Post("/panel-background", handleSetPanelBackground)
		r.Post("/panel-background/upload", handleUploadPanelBackground)
		r.Get("/panel-background/image", handleGetPanelBackgroundImage)
		r.Get("/panel-background/history", handleListPanelBackgroundHistory)
		r.Get("/panel-background/history/{id}", handleGetPanelBackgroundHistoryImage)
		r.Delete("/panel-background/history", handleClearPanelBackgroundHistory)
		r.Delete("/panel-background/history/{id}", handleDeletePanelBackgroundHistory)
		r.Get("/text-color", handleGetTextColor)
		r.Post("/text-color", handleSetTextColor)
	})
}

func handleGetTextColor(w http.ResponseWriter, r *http.Request) {
	settings, err := loadTextColorSettings()
	if err != nil {
		mlog.L().Warn("load text color settings failed, fallback to defaults", zap.Error(err))
		settings = defaultTextColorSettings()
	}
	writeJSON(w, http.StatusOK, settings)
}

func handleSetTextColor(w http.ResponseWriter, r *http.Request) {
	var payload textColorSettings
	if err := json.NewDecoder(r.Body).Decode(&payload); err != nil {
		writeError(w, http.StatusBadRequest, fmt.Errorf("invalid request body: %w", err))
		return
	}

	settings := normalizeTextColorSettings(payload)
	if err := saveTextColorSettings(settings); err != nil {
		writeError(w, http.StatusInternalServerError, fmt.Errorf("save text color settings failed: %w", err))
		return
	}
	writeJSON(w, http.StatusOK, settings)
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
	if settings.Mode == "color" && strings.TrimSpace(settings.Color) == "" {
		writeError(w, http.StatusBadRequest, errors.New("纯色背景不能为空"))
		return
	}
	if settings.Mode == "upload" {
		if settings.UploadID != "" {
			if _, _, err := findPanelBackgroundHistoryImageByID(settings.UploadID); err != nil {
				writeError(w, http.StatusBadRequest, errors.New("所选历史背景图片不存在，请重新选择"))
				return
			}
		} else {
			if _, err := os.Stat(panelBackgroundImagePath()); err != nil {
				writeError(w, http.StatusBadRequest, errors.New("请先上传本地图片，再应用本地背景"))
				return
			}
		}
		settings.URL = ""
	} else {
		settings.UploadID = ""
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

	uploadID := generatePanelBackgroundUploadID()
	extension := panelBackgroundImageExtension(contentType)
	if err := os.MkdirAll(panelBackgroundHistoryDirPath(), 0o755); err != nil {
		writeError(w, http.StatusInternalServerError, fmt.Errorf("创建背景历史目录失败: %w", err))
		return
	}
	historyPath := panelBackgroundHistoryFilePath(uploadID, extension)
	if err := writeManagedFile(historyPath, data, nil, nil, nil); err != nil {
		writeError(w, http.StatusInternalServerError, fmt.Errorf("保存背景历史图片失败: %w", err))
		return
	}

	if err := writeManagedFile(panelBackgroundImagePath(), data, nil, nil, nil); err != nil {
		writeError(w, http.StatusInternalServerError, fmt.Errorf("保存上传图片失败: %w", err))
		return
	}

	historyInfo, _ := os.Stat(historyPath)
	imageURL := resolvePanelBackgroundHistoryImageURL(uploadID, historyInfo)
	writeJSON(w, http.StatusOK, map[string]any{
		"message":   "上传成功",
		"upload_id": uploadID,
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

func handleListPanelBackgroundHistory(w http.ResponseWriter, r *http.Request) {
	items, err := listPanelBackgroundHistoryItems()
	if err != nil {
		writeError(w, http.StatusInternalServerError, fmt.Errorf("读取背景历史失败: %w", err))
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{
		"items": items,
	})
}

func handleGetPanelBackgroundHistoryImage(w http.ResponseWriter, r *http.Request) {
	id := normalizePanelBackgroundUploadID(chi.URLParam(r, "id"))
	if id == "" {
		writeError(w, http.StatusBadRequest, errors.New("无效的历史背景 ID"))
		return
	}
	imagePath, info, err := findPanelBackgroundHistoryImageByID(id)
	if err != nil {
		writeError(w, http.StatusNotFound, errors.New("未找到历史背景图片"))
		return
	}
	data, err := os.ReadFile(imagePath)
	if err != nil {
		writeError(w, http.StatusInternalServerError, fmt.Errorf("读取背景图片失败: %w", err))
		return
	}
	if len(data) == 0 {
		writeError(w, http.StatusNotFound, errors.New("背景图片为空"))
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

func handleDeletePanelBackgroundHistory(w http.ResponseWriter, r *http.Request) {
	id := normalizePanelBackgroundUploadID(chi.URLParam(r, "id"))
	if id == "" {
		writeError(w, http.StatusBadRequest, errors.New("无效的历史背景 ID"))
		return
	}

	path, _, err := findPanelBackgroundHistoryImageByID(id)
	if err != nil {
		writeError(w, http.StatusNotFound, errors.New("未找到历史背景图片"))
		return
	}
	if err := os.Remove(path); err != nil {
		writeError(w, http.StatusInternalServerError, fmt.Errorf("删除背景历史失败: %w", err))
		return
	}

	if settings, err := loadPanelBackgroundSettings(); err == nil && settings.Mode == "upload" && settings.UploadID == id {
		settings.Mode = "none"
		settings.UploadID = ""
		settings.URL = ""
		_ = savePanelBackgroundSettings(settings)
	}

	writeJSON(w, http.StatusOK, map[string]any{
		"message": "历史背景已删除",
	})
}

func handleClearPanelBackgroundHistory(w http.ResponseWriter, r *http.Request) {
	dir := panelBackgroundHistoryDirPath()
	entries, err := os.ReadDir(dir)
	if err != nil {
		if os.IsNotExist(err) {
			writeJSON(w, http.StatusOK, map[string]any{
				"message": "历史背景为空",
				"count":   0,
			})
			return
		}
		writeError(w, http.StatusInternalServerError, fmt.Errorf("读取背景历史目录失败: %w", err))
		return
	}
	removed := 0
	for _, entry := range entries {
		if entry.IsDir() {
			continue
		}
		if err := os.Remove(filepath.Join(dir, entry.Name())); err == nil {
			removed++
		}
	}

	if settings, err := loadPanelBackgroundSettings(); err == nil && settings.Mode == "upload" && settings.UploadID != "" {
		settings.Mode = "none"
		settings.UploadID = ""
		settings.URL = ""
		_ = savePanelBackgroundSettings(settings)
	}

	writeJSON(w, http.StatusOK, map[string]any{
		"message": "历史背景已清空",
		"count":   removed,
	})
}

func defaultPanelBackgroundSettings() panelBackgroundSettings {
	return panelBackgroundSettings{
		Mode:       "none",
		URL:        "",
		Color:      "",
		LightColor: "#f8fafc",
		DarkColor:  "#0f172a",
		UploadID:   "",
		Opacity:    0.9,
		Blur:       10,
	}
}

func normalizePanelBackgroundUploadID(raw string) string {
	v := strings.TrimSpace(raw)
	if !panelBackgroundUploadIDRegexp.MatchString(v) {
		return ""
	}
	return v
}

func normalizePanelBackgroundSettings(raw panelBackgroundSettings) panelBackgroundSettings {
	out := defaultPanelBackgroundSettings()
	switch strings.ToLower(strings.TrimSpace(raw.Mode)) {
	case "url":
		out.Mode = "url"
	case "upload":
		out.Mode = "upload"
	case "color":
		out.Mode = "color"
	default:
		out.Mode = "none"
	}
	out.URL = strings.TrimSpace(raw.URL)
	out.Color = normalizeHexColor(raw.Color, "")
	out.LightColor = normalizeHexColor(raw.LightColor, "")
	out.DarkColor = normalizeHexColor(raw.DarkColor, "")
	if out.LightColor == "" {
		out.LightColor = normalizeHexColor(out.Color, "#f8fafc")
	}
	if out.DarkColor == "" {
		out.DarkColor = normalizeHexColor(out.Color, "#0f172a")
	}
	if out.Mode == "color" && out.Color == "" {
		out.Color = out.DarkColor
	}
	out.UploadID = normalizePanelBackgroundUploadID(raw.UploadID)
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
		Mode:       settings.Mode,
		URL:        settings.URL,
		Color:      normalizeHexColor(settings.LightColor, settings.Color),
		LightColor: normalizeHexColor(settings.LightColor, "#f8fafc"),
		DarkColor:  normalizeHexColor(settings.DarkColor, "#0f172a"),
		UploadID:   settings.UploadID,
		Opacity:    settings.Opacity,
		Blur:       settings.Blur,
	}
	switch settings.Mode {
	case "url":
		resp.ImageURL = settings.URL
	case "upload":
		if settings.UploadID != "" {
			_, info, err := findPanelBackgroundHistoryImageByID(settings.UploadID)
			if err == nil {
				resp.ImageURL = resolvePanelBackgroundHistoryImageURL(settings.UploadID, info)
			}
		}
		if resp.ImageURL == "" {
			resp.ImageURL = resolveUploadedPanelBackgroundURL()
		}
	}
	return resp
}

func defaultTextColorSettings() textColorSettings {
	return textColorSettings{
		Light: textColorSetting{Mode: "default", Color: "#1e252b"},
		Dark:  textColorSetting{Mode: "default", Color: "#f8fafc"},
	}
}

func normalizeHexColor(raw string, fallback string) string {
	v := strings.TrimSpace(raw)
	if v == "" {
		return strings.ToLower(fallback)
	}
	match := hexColorRegexp.FindStringSubmatch(v)
	if len(match) != 2 {
		return strings.ToLower(fallback)
	}
	return "#" + strings.ToLower(match[1])
}

func normalizeTextColorSetting(raw textColorSetting, theme string) textColorSetting {
	defaults := defaultTextColorSettings()
	base := defaults.Light
	if theme == "dark" {
		base = defaults.Dark
	}

	mode := strings.ToLower(strings.TrimSpace(raw.Mode))
	switch mode {
	case "custom":
		return textColorSetting{
			Mode:  "custom",
			Color: normalizeHexColor(raw.Color, base.Color),
		}
	default:
		return textColorSetting{
			Mode:  "default",
			Color: base.Color,
		}
	}
}

func normalizeTextColorSettings(raw textColorSettings) textColorSettings {
	return textColorSettings{
		Light: normalizeTextColorSetting(raw.Light, "light"),
		Dark:  normalizeTextColorSetting(raw.Dark, "dark"),
	}
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

func panelBackgroundHistoryDirPath() string {
	base := MainConfigBaseDir
	if strings.TrimSpace(base) == "" {
		base = "."
	}
	return filepath.Join(base, panelBackgroundHistoryRel)
}

func panelBackgroundImageExtension(contentType string) string {
	switch strings.ToLower(strings.TrimSpace(contentType)) {
	case "image/png":
		return "png"
	case "image/jpeg":
		return "jpg"
	case "image/gif":
		return "gif"
	case "image/webp":
		return "webp"
	case "image/bmp":
		return "bmp"
	case "image/svg+xml":
		return "svg"
	default:
		return "bin"
	}
}

func generatePanelBackgroundUploadID() string {
	return fmt.Sprintf("%x", time.Now().UTC().UnixNano())
}

func panelBackgroundHistoryFilePath(uploadID, extension string) string {
	ext := strings.TrimPrefix(strings.TrimSpace(extension), ".")
	if ext == "" {
		ext = "bin"
	}
	return filepath.Join(panelBackgroundHistoryDirPath(), fmt.Sprintf("%s.%s", uploadID, ext))
}

func findPanelBackgroundHistoryImageByID(uploadID string) (string, os.FileInfo, error) {
	id := normalizePanelBackgroundUploadID(uploadID)
	if id == "" {
		return "", nil, errors.New("invalid id")
	}
	entries, err := os.ReadDir(panelBackgroundHistoryDirPath())
	if err != nil {
		return "", nil, err
	}
	prefix := id + "."
	for _, entry := range entries {
		if entry.IsDir() {
			continue
		}
		name := entry.Name()
		if !strings.HasPrefix(name, prefix) {
			continue
		}
		path := filepath.Join(panelBackgroundHistoryDirPath(), name)
		info, statErr := os.Stat(path)
		if statErr != nil {
			return "", nil, statErr
		}
		return path, info, nil
	}
	return "", nil, os.ErrNotExist
}

func resolvePanelBackgroundHistoryImageURL(uploadID string, info os.FileInfo) string {
	id := normalizePanelBackgroundUploadID(uploadID)
	if id == "" {
		return ""
	}
	version := int64(0)
	if info != nil {
		version = info.ModTime().UnixNano()
	}
	if version > 0 {
		return fmt.Sprintf("/api/v1/appearance/panel-background/history/%s?v=%d", id, version)
	}
	return fmt.Sprintf("/api/v1/appearance/panel-background/history/%s", id)
}

func textColorSettingsPath() string {
	base := MainConfigBaseDir
	if strings.TrimSpace(base) == "" {
		base = "."
	}
	return filepath.Join(base, appearanceTextSettingsFile)
}

func resolveUploadedPanelBackgroundURL() string {
	info, err := os.Stat(panelBackgroundImagePath())
	if err != nil {
		return ""
	}
	return fmt.Sprintf("/api/v1/appearance/panel-background/image?v=%d", info.ModTime().UnixNano())
}

func listPanelBackgroundHistoryItems() ([]panelBackgroundHistoryItem, error) {
	entries, err := os.ReadDir(panelBackgroundHistoryDirPath())
	if err != nil {
		if os.IsNotExist(err) {
			return []panelBackgroundHistoryItem{}, nil
		}
		return nil, err
	}

	items := make([]panelBackgroundHistoryItem, 0, len(entries))
	for _, entry := range entries {
		if entry.IsDir() {
			continue
		}
		name := entry.Name()
		dot := strings.IndexByte(name, '.')
		if dot <= 0 {
			continue
		}
		id := normalizePanelBackgroundUploadID(name[:dot])
		if id == "" {
			continue
		}
		path := filepath.Join(panelBackgroundHistoryDirPath(), name)
		info, statErr := os.Stat(path)
		if statErr != nil {
			continue
		}
		items = append(items, panelBackgroundHistoryItem{
			ID:         id,
			ImageURL:   resolvePanelBackgroundHistoryImageURL(id, info),
			Size:       info.Size(),
			ModifiedAt: info.ModTime().UTC().Format(time.RFC3339),
		})
	}

	sort.Slice(items, func(i, j int) bool {
		return items[i].ModifiedAt > items[j].ModifiedAt
	})
	return items, nil
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

func loadTextColorSettings() (textColorSettings, error) {
	path := textColorSettingsPath()
	raw, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return defaultTextColorSettings(), nil
		}
		return defaultTextColorSettings(), err
	}
	var parsed textColorSettings
	if err := json.Unmarshal(raw, &parsed); err != nil {
		return defaultTextColorSettings(), err
	}
	return normalizeTextColorSettings(parsed), nil
}

func saveTextColorSettings(settings textColorSettings) error {
	normalized := normalizeTextColorSettings(settings)
	data, err := json.MarshalIndent(normalized, "", "  ")
	if err != nil {
		return err
	}
	return writeManagedFile(textColorSettingsPath(), data, func(raw []byte) error {
		var parsed textColorSettings
		if err := json.Unmarshal(raw, &parsed); err != nil {
			return err
		}
		_ = normalizeTextColorSettings(parsed)
		return nil
	}, nil, nil)
}
