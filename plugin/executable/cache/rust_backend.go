package cache

import (
	"errors"
	"os"
	"strings"
	"time"

	"go.uber.org/zap"
)

const rustCacheBackendEnv = "MOSDNS_CACHE_BACKEND"

var errRustCacheUnavailable = errors.New("rust cache backend is unavailable in this build")

type rustCacheLookupState uint8

const (
	rustCacheMiss rustCacheLookupState = iota
	rustCacheFresh
	rustCacheLazy
)

type rustCacheLookupResult struct {
	State     rustCacheLookupState
	Response  []byte
	DomainSet string
	StoredAt  time.Time
}

type rustCacheBackend interface {
	Name() string
	Lookup(key []byte, now time.Time) (rustCacheLookupResult, error)
	Store(key []byte, value *item, cacheExpiresAt time.Time) error
	Len() (int, error)
	Flush() error
	Close() error
}

type rustCacheBackendHolder struct {
	backend rustCacheBackend
}

var rustBackendFactory = newRustCacheBackend

// openRustCacheBackend returns requested=true only when the operator explicitly
// selected Rust. A failed request is observable and leaves the Go backend active.
func openRustCacheBackend(args *Args, logger *zap.Logger) (backend rustCacheBackend, requested bool) {
	if strings.ToLower(strings.TrimSpace(os.Getenv(rustCacheBackendEnv))) != "rust" {
		return nil, false
	}
	backend, err := rustBackendFactory(args)
	if err != nil {
		logger.Warn("failed to initialize experimental rust cache; using go backend",
			zap.String("env", rustCacheBackendEnv),
			zap.Error(err))
		return nil, true
	}
	logger.Info("experimental rust cache enabled",
		zap.String("backend", backend.Name()),
		zap.String("env", rustCacheBackendEnv))
	return backend, true
}

func (c *Cache) rustActive() bool {
	return c.rustBackend.Load() != nil
}

func (c *Cache) lookupRust(key []byte, now time.Time) (rustCacheLookupResult, bool) {
	holder := c.rustBackend.Load()
	if holder == nil {
		return rustCacheLookupResult{}, false
	}
	result, err := holder.backend.Lookup(key, now)
	if err != nil {
		c.disableRustBackend("lookup", err)
		return rustCacheLookupResult{}, false
	}
	return result, true
}

func (c *Cache) storeRust(key []byte, value *item, cacheExpiresAt time.Time) {
	if value == nil {
		return
	}
	holder := c.rustBackend.Load()
	if holder == nil {
		return
	}
	err := holder.backend.Store(key, value, cacheExpiresAt)
	if err != nil {
		c.disableRustBackend("store", err)
	}
}

func (c *Cache) currentCacheLen() int {
	holder := c.rustBackend.Load()
	if holder == nil {
		return c.backend.Len()
	}
	n, err := holder.backend.Len()
	if err != nil {
		c.disableRustBackend("len", err)
		return c.backend.Len()
	}
	return n
}

func (c *Cache) flushRust() {
	holder := c.rustBackend.Load()
	if holder == nil {
		return
	}
	err := holder.backend.Flush()
	if err != nil {
		c.disableRustBackend("flush", err)
	}
}

func (c *Cache) closeRust() {
	holder := c.rustBackend.Swap(nil)
	if holder != nil {
		if err := holder.backend.Close(); err != nil {
			c.logger.Warn("failed to close experimental rust cache", zap.Error(err))
		}
	}
}

func (c *Cache) disableRustBackend(operation string, cause error) {
	holder := c.rustBackend.Swap(nil)
	if holder == nil {
		return
	}
	c.logger.Warn("experimental rust cache failed; circuit breaker selected go backend",
		zap.String("operation", operation),
		zap.String("backend", holder.backend.Name()),
		zap.Error(cause))
	if err := holder.backend.Close(); err != nil {
		c.logger.Warn("failed to close disabled rust cache", zap.Error(err))
	}
}

func (c *Cache) seedRustFromGo() error {
	holder := c.rustBackend.Load()
	if holder == nil {
		return nil
	}
	err := c.backend.Range(func(k key, value *item, cacheExpiresAt time.Time) error {
		return holder.backend.Store([]byte(k), value, cacheExpiresAt)
	})
	return err
}
