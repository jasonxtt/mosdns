package coremain

import (
	"sync"
	"sync/atomic"
	"time"

	"go.uber.org/zap"
	"go.uber.org/zap/zapcore"
)

// MODIFIED: Exported LogMarkKey to be accessible from other packages.
const LogMarkKey = 0xFEEDBEEF // ADDED: A globally accessible key for marking contexts.

// InMemoryLogCollector is a thread-safe in-memory log collector.
type InMemoryLogCollector struct {
	mu        sync.Mutex
	capturing atomic.Bool
	logs      []map[string]interface{}
	stopTimer *time.Timer
}

// GlobalLogCollector is a singleton instance for log capturing.
var GlobalLogCollector = &InMemoryLogCollector{
	logs: make([]map[string]interface{}, 0, 2048), // Pre-allocate capacity
}

// StartCapture begins capturing logs for a given duration.
func (c *InMemoryLogCollector) StartCapture(duration time.Duration, logLevel zap.AtomicLevel) {
	c.mu.Lock()
	defer c.mu.Unlock()

	if c.capturing.Load() {
		c.stopTimer.Stop() // Stop previous timer if a new capture starts
	}

	// Reset log buffer for the new capture session.
	c.logs = make([]map[string]interface{}, 0, 2048)
	
	c.capturing.Store(true)
	logLevel.SetLevel(zap.DebugLevel) // Switch to DEBUG level

	c.stopTimer = time.AfterFunc(duration, func() {
		c.StopCapture(logLevel)
	})
}

// StopCapture stops the log capture and resets the log level.
func (c *InMemoryLogCollector) StopCapture(logLevel zap.AtomicLevel) {
	c.mu.Lock()
	defer c.mu.Unlock()

	if !c.capturing.Load() {
		return
	}

	logLevel.SetLevel(zap.InfoLevel) // Restore to default INFO level
	c.capturing.Store(false)
	if c.stopTimer != nil {
		c.stopTimer.Stop()
	}
}

// AddLog adds a structured log entry to the in-memory buffer if capturing is active.
func (c *InMemoryLogCollector) AddLog(entry zapcore.Entry, fields[]zapcore.Field) {
	if !c.capturing.Load() {
		return
	}
    
	if entry.Level != zap.DebugLevel {
		return
	}

	c.mu.Lock()
	defer c.mu.Unlock()

	if !c.capturing.Load() {
		return
	}

	// Convert fields to a map
	enc := zapcore.NewMapObjectEncoder()
	for _, f := range fields {
		f.AddTo(enc)
	}

	// Add standard entry fields
	logMap := enc.Fields
	logMap["level"] = entry.Level.String()
	logMap["time"] = entry.Time
	logMap["msg"] = entry.Message
	logMap["logger_name"] = entry.LoggerName

	c.logs = append(c.logs, logMap)
}

// GetLogs returns all captured logs and clears the in-memory buffer.
// This is a "read-once" operation.
func (c *InMemoryLogCollector) GetLogs() []map[string]interface{} {
	c.mu.Lock()
	defer c.mu.Unlock()

	// Efficiently swap out the logs buffer with a new empty one.
	// This avoids holding the lock while copying and is very fast.
	logsToReturn := c.logs
	c.logs = make([]map[string]interface{}, 0, 2048)

	return logsToReturn
}

// TeeCore is a custom zapcore.Core that writes to two destinations:
// 1. The original core (e.g., a file or console)
// 2. The in-memory log collector
type TeeCore struct {
	zapcore.Core
	collector *InMemoryLogCollector
}

// NewTeeCore creates a new TeeCore.
func NewTeeCore(core zapcore.Core, collector *InMemoryLogCollector) *TeeCore {
	return &TeeCore{
		Core:      core,
		collector: collector,
	}
}

// Write duplicates the log entry to both the original core and the collector.
func (t *TeeCore) Write(entry zapcore.Entry, fields []zapcore.Field) error {
	// Add log to our in-memory collector
	t.collector.AddLog(entry, fields)

	// Pass the log to the original core (e.g., to write to file/console)
	return t.Core.Write(entry, fields)
}

// Check decides if a log entry should be processed.
func (t *TeeCore) Check(ent zapcore.Entry, ce *zapcore.CheckedEntry) *zapcore.CheckedEntry {
	if t.Enabled(ent.Level) {
		return ce.AddCore(ent, t)
	}
	return ce
}
