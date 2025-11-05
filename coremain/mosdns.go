/*
 * Copyright (C) 2020-2022, IrineSistiana
 *
 * This file is part of mosdns.
 *
 * mosdns is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * mosdns is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program.  If not, see <https://www.gnu.org/licenses/>.
 */

package coremain

import (
	"bytes"
	"embed"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/http/pprof"
	"path"
	"path/filepath"
	"strings"

	"github.com/IrineSistiana/mosdns/v5/mlog"
	"github.com/IrineSistiana/mosdns/v5/pkg/safe_close"
	"github.com/go-chi/chi/v5"
	"github.com/prometheus/client_golang/prometheus"
	"github.com/prometheus/client_golang/prometheus/collectors"
	"github.com/prometheus/client_golang/prometheus/promhttp"
	"go.uber.org/zap"
)

//go:embed www/*
var content embed.FS

type Mosdns struct {
	logger *zap.Logger // non-nil logger.

	// Plugins
	plugins map[string]any

	httpMux    *chi.Mux
	metricsReg *prometheus.Registry
	sc         *safe_close.SafeClose
}

// NewMosdns initializes a mosdns instance and its plugins.
func NewMosdns(cfg *Config) (*Mosdns, error) {
	// Init logger.
	baseLogger, err := mlog.NewLogger(cfg.Log)
	if err != nil {
		return nil, fmt.Errorf("failed to init logger: %w", err)
	}

	// Create our TeeCore to also write to the in-memory collector for detailed process logs.
	teeCore := NewTeeCore(baseLogger.Core(), GlobalLogCollector)

	// Create the final logger with our TeeCore.
	lg := zap.New(teeCore, zap.AddCaller(), zap.AddStacktrace(zap.ErrorLevel))

	// Start the audit log collector's background worker.
	GlobalAuditCollector.StartWorker()

	m := &Mosdns{
		logger:     lg,
		plugins:    make(map[string]any),
		httpMux:    chi.NewRouter(),
		metricsReg: newMetricsReg(),
		sc:         safe_close.NewSafeClose(),
	}

	// This must be called after m.httpMux and m.metricsReg been set.
	m.initHttpMux()

	// Register our new APIs.
	RegisterCaptureAPI(m.httpMux) // For process logs
	RegisterAuditAPI(m.httpMux)   // For audit logs v1
	RegisterAuditAPIV2(m.httpMux) // For audit logs v2
	RegisterUpdateAPI(m.httpMux)  // For binary updates

	// Start http api server
	if httpAddr := cfg.API.HTTP; len(httpAddr) > 0 {
		httpServer := &http.Server{
			Addr:    httpAddr,
			Handler: m.httpMux,
		}
		m.sc.Attach(func(done func(), closeSignal <-chan struct{}) {
			defer done()
			errChan := make(chan error, 1)
			go func() {
				m.logger.Info("starting api http server", zap.String("addr", httpAddr))
				errChan <- httpServer.ListenAndServe()
			}()
			select {
			case err := <-errChan:
				m.sc.SendCloseSignal(err)
			case <-closeSignal:
				_ = httpServer.Close()
			}
		})
	}

	// Load plugins.

	// Close all plugins on signal.
	m.sc.Attach(func(done func(), closeSignal <-chan struct{}) {
		go func() {
			defer done()
			<-closeSignal

			// Stop the audit worker gracefully.
			GlobalAuditCollector.StopWorker()

			m.logger.Info("starting shutdown sequences")
			for tag, p := range m.plugins {
				if closer, _ := p.(io.Closer); closer != nil {
					m.logger.Info("closing plugin", zap.String("tag", tag))
					_ = closer.Close()
				}
			}
			m.logger.Info("all plugins were closed")
		}()
	})

	// Preset plugins
	if err := m.loadPresetPlugins(); err != nil {
		m.sc.SendCloseSignal(err)
		_ = m.sc.WaitClosed()
		return nil, err
	}
	// Plugins from config.
	if err := m.loadPluginsFromCfg(cfg, 0); err != nil {
		m.sc.SendCloseSignal(err)
		_ = m.sc.WaitClosed()
		return nil, err
	}
	m.logger.Info("all plugins are loaded")

	return m, nil
}

// NewTestMosdnsWithPlugins returns a mosdns instance for testing.
func NewTestMosdnsWithPlugins(p map[string]any) *Mosdns {
	return &Mosdns{
		logger:     mlog.Nop(),
		httpMux:    chi.NewRouter(),
		plugins:    p,
		metricsReg: newMetricsReg(),
		sc:         safe_close.NewSafeClose(),
	}
}

func (m *Mosdns) GetSafeClose() *safe_close.SafeClose {
	return m.sc
}

// CloseWithErr is a shortcut for m.sc.SendCloseSignal
func (m *Mosdns) CloseWithErr(err error) {
	m.sc.SendCloseSignal(err)
}

// Logger returns a non-nil logger.
func (m *Mosdns) Logger() *zap.Logger {
	return m.logger
}

// GetPlugin returns a plugin.
func (m *Mosdns) GetPlugin(tag string) any {
	return m.plugins[tag]
}

// GetMetricsReg returns a prometheus.Registerer with a prefix of "mosdns_"
func (m *Mosdns) GetMetricsReg() prometheus.Registerer {
	return prometheus.WrapRegistererWithPrefix("mosdns_", m.metricsReg)
}

func (m *Mosdns) GetAPIRouter() *chi.Mux {
	return m.httpMux
}

func (m *Mosdns) RegPluginAPI(tag string, mux *chi.Mux) {
	m.httpMux.Mount("/plugins/"+tag, mux)
}

func newMetricsReg() *prometheus.Registry {
	reg := prometheus.NewRegistry()
	reg.MustRegister(collectors.NewProcessCollector(collectors.ProcessCollectorOpts{}))
	reg.MustRegister(collectors.NewGoCollector())
	return reg
}

// initHttpMux initializes api entries. It MUST be called after m.metricsReg being initialized.
func (m *Mosdns) initHttpMux() {
	// 全局 CORS 中间件
	corsMiddleware := func(next http.Handler) http.Handler {
		return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			w.Header().Set("Access-Control-Allow-Origin", "*")
			// <<< MODIFIED: Allow PUT and DELETE methods for plugin APIs
			w.Header().Set("Access-Control-Allow-Methods", "GET, POST, OPTIONS, PUT, DELETE")
			w.Header().Set("Access-Control-Allow-Headers", "Content-Type, Accept")

			if r.Method == http.MethodOptions {
				w.WriteHeader(http.StatusOK)
				return
			}
			next.ServeHTTP(w, r)
		})
	}

	m.httpMux.Use(corsMiddleware)

	// metrics 处理 (只注册一次)
	metricsHandler := promhttp.HandlerFor(m.metricsReg, promhttp.HandlerOpts{})
	wrappedMetricsHandler := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		m.logger.Debug("Metrics endpoint accessed",
			zap.String("remote_addr", r.RemoteAddr),
			zap.String("method", r.Method))
		metricsHandler.ServeHTTP(w, r)
	})
	m.httpMux.Method(http.MethodGet, "/metrics", wrappedMetricsHandler)

	// [修改] 将原来的公共handler拆分为两个独立的handler

	// [新增] 根路由 ("/") 的 handler，指向 mosdnsp.html
	rootHandler := func(w http.ResponseWriter, r *http.Request) {
		data, err := content.ReadFile("www/mosdnsp.html") // 读取新文件
		if err != nil {
			m.logger.Error("Error reading embedded file", zap.String("file", "www/mosdnsp.html"), zap.Error(err))
			http.Error(w, "Error reading the embedded file", http.StatusInternalServerError)
			return
		}
		w.Header().Set("Content-Type", "text/html; charset=utf-8")
		if _, err := w.Write(data); err != nil {
			m.logger.Error("Error writing response", zap.Error(err))
		}
	}

	// [新增] graphic 路由 ("/graphic") 的 handler，保持指向 mosdns.html
	graphicHandler := func(w http.ResponseWriter, r *http.Request) {
		data, err := content.ReadFile("www/mosdns.html") // 读取原文件
		if err != nil {
			m.logger.Error("Error reading embedded file", zap.String("file", "www/mosdns.html"), zap.Error(err))
			http.Error(w, "Error reading the embedded file", http.StatusInternalServerError)
			return
		}
		w.Header().Set("Content-Type", "text/html; charset=utf-8")
		if _, err := w.Write(data); err != nil {
			m.logger.Error("Error writing response", zap.Error(err))
		}
	}

	// [新增] log 路由 ("/log") 的 handler, 指向 /www/log.html
	logHandler := func(w http.ResponseWriter, r *http.Request) {
		data, err := content.ReadFile("www/log.html") // 读取 /www/log.html
		if err != nil {
			m.logger.Error("Error reading embedded file", zap.String("file", "www/log.html"), zap.Error(err))
			http.Error(w, "Error reading the embedded file", http.StatusInternalServerError)
			return
		}
		w.Header().Set("Content-Type", "text/html; charset=utf-8")
		if _, err := w.Write(data); err != nil {
			m.logger.Error("Error writing response", zap.Error(err))
		}
	}

	plainLogHandler := func(w http.ResponseWriter, r *http.Request) {
		data, err := content.ReadFile("www/log_plain.html")
		if err != nil {
			m.logger.Error("Error reading embedded file", zap.String("file", "www/log_plain.html"), zap.Error(err))
			http.Error(w, "Error reading the embedded file", http.StatusInternalServerError)
			return
		}
		w.Header().Set("Content-Type", "text/html; charset=utf-8")
		if _, err := w.Write(data); err != nil {
			m.logger.Error("Error writing response", zap.Error(err))
		}
	}

	redirectToLog := func(w http.ResponseWriter, r *http.Request) {
		http.Redirect(w, r, "/log", http.StatusFound)
	}

	staticAssetHandler := func(w http.ResponseWriter, r *http.Request) {
		relativePath := strings.TrimPrefix(path.Clean(r.URL.Path), "/")
		if !strings.HasPrefix(relativePath, "assets/") {
			http.NotFound(w, r)
			return
		}
		filePath := path.Join("www", relativePath)
		data, err := content.ReadFile(filePath)
		if err != nil {
			m.logger.Error("Error reading embedded static file", zap.String("path", filePath), zap.Error(err))
			http.NotFound(w, r)
			return
		}

		switch ext := path.Ext(filePath); ext {
		case ".css":
			w.Header().Set("Content-Type", "text/css; charset=utf-8")
		case ".js":
			w.Header().Set("Content-Type", "application/javascript; charset=utf-8")
		case ".woff2":
			w.Header().Set("Content-Type", "font/woff2")
		case ".woff":
			w.Header().Set("Content-Type", "font/woff")
		case ".ttf":
			w.Header().Set("Content-Type", "font/ttf")
		}

		if _, err := w.Write(data); err != nil {
			m.logger.Error("Error writing static asset response", zap.Error(err))
		}
	}

	// [修改] 为每个路由注册对应的 handler
	m.httpMux.Get("/", rootHandler)
	m.httpMux.Get("/graphic", graphicHandler)
	m.httpMux.Get("/log", logHandler)
	m.httpMux.Get("/plog", plainLogHandler)
	m.httpMux.Get("/rlog", redirectToLog)
	m.httpMux.Get("/assets/*", staticAssetHandler)

	// Register pprof.
	m.httpMux.Route("/debug/pprof", func(r chi.Router) {
		r.Get("/*", pprof.Index)
		r.Get("/cmdline", pprof.Cmdline)
		r.Get("/profile", pprof.Profile)
		r.Get("/symbol", pprof.Symbol)
		r.Get("/trace", pprof.Trace)
	})

	// A helper page for invalid request.
	invalidApiReqHelper := func(w http.ResponseWriter, req *http.Request) {
		b := new(bytes.Buffer)
		_, _ = fmt.Fprintf(b, "Invalid request %s %s\n\n", req.Method, req.RequestURI)
		b.WriteString("Available api urls:\n")
		_ = chi.Walk(m.httpMux, func(method string, route string, handler http.Handler, middlewares ...func(http.Handler) http.Handler) error {
			b.WriteString(method)
			b.WriteByte(' ')
			b.WriteString(route)
			b.WriteByte('\n')
			return nil
		})
		_, _ = w.Write(b.Bytes())
	}
	m.httpMux.NotFound(invalidApiReqHelper)
	m.httpMux.MethodNotAllowed(invalidApiReqHelper)
}

func (m *Mosdns) loadPresetPlugins() error {
	for tag, f := range LoadNewPersetPluginFuncs() {
		p, err := f(NewBP(tag, m))
		if err != nil {
			return fmt.Errorf("failed to init preset plugin %s, %w", tag, err)
		}
		m.plugins[tag] = p
	}
	return nil
}

// loadPluginsFromCfg loads plugins from this config. It follows include first.
func (m *Mosdns) loadPluginsFromCfg(cfg *Config, includeDepth int) error {
	const maxIncludeDepth = 8
	if includeDepth > maxIncludeDepth {
		return errors.New("maximum include depth reached")
	}
	includeDepth++

	// Follow include first.
	for _, includePath := range cfg.Include {
		resolvedPath := includePath
		if len(cfg.baseDir) > 0 && !filepath.IsAbs(includePath) {
			resolvedPath = filepath.Join(cfg.baseDir, includePath)
		}
		subCfg, path, err := loadConfig(resolvedPath)
		if err != nil {
			return fmt.Errorf("failed to read config from %s, %w", includePath, err)
		}
		m.logger.Info("load config", zap.String("file", path))
		if err := m.loadPluginsFromCfg(subCfg, includeDepth); err != nil {
			return fmt.Errorf("failed to load config from %s, %w", includePath, err)
		}
	}

	for i, pc := range cfg.Plugins {
		if err := m.newPlugin(pc); err != nil {
			return fmt.Errorf("failed to init plugin #%d %s, %w", i, pc.Tag, err)
		}
	}
	return nil
}
