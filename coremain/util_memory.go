package coremain

import (
	"net/http"
	"runtime/debug"
	"time"
)

// ManualGC 手动触发 GC，用于清理大量临时内存。
// 异步执行，带有短暂延迟，避免阻塞主流程。
func ManualGC() {
	go func() {
		time.Sleep(200 * time.Millisecond)
		debug.FreeOSMemory()
	}()
}

// WithAsyncGC 是一个 HTTP 中间件/包装器。
// 它可以包裹任何 http.HandlerFunc，在请求结束后自动触发 ManualGC。
// 这里的 handler 和返回值类型都在 coremain 包内，可以直接调用。
func WithAsyncGC(handler http.HandlerFunc) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		// 注册清理逻辑
		defer ManualGC()
		// 执行原始逻辑
		handler(w, r)
	}
}
