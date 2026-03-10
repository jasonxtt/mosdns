package coremain

import (
	"net/http"
)

// ManualGC 被修改为空函数。
// 外部原有的调用依然可以正常执行，但不会再有任何实际的 GC 或休眠操作。
func ManualGC() {
}

// WithAsyncGC 被修改为透明包装器（直接返回原 handler）。
// 外部路由注册调用它时不会报错，但去除了原有的 defer ManualGC() 逻辑。
func WithAsyncGC(handler http.HandlerFunc) http.HandlerFunc {
	return handler
}
