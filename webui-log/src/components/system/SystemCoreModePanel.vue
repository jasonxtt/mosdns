<script setup>
defineProps({
  coreMode: {
    type: String,
    default: ''
  },
  layout: {
    type: String,
    default: 'buttons'
  },
  switchLoading: {
    type: Object,
    required: true
  }
})

defineEmits(['set-core-mode'])
</script>

<template>
  <section class="panel control-module system-grid-dual-item">
    <header class="module-head">
      <div>
        <h3>核心运行模式</h3>
      </div>
      <div v-if="layout === 'buttons'" class="actions">
        <button class="btn tiny core-mode-btn" :class="coreMode === 'A' ? 'primary is-active' : 'secondary'" :disabled="switchLoading.switch3" @click="$emit('set-core-mode', 'A')">兼容模式</button>
        <button class="btn tiny core-mode-btn" :class="coreMode === 'B' ? 'primary is-active' : 'secondary'" :disabled="switchLoading.switch3" @click="$emit('set-core-mode', 'B')">安全模式</button>
      </div>
    </header>

    <div v-if="layout === 'cards'" class="system-core-mode-grid">
      <button
        class="system-core-mode-option"
        :class="{ active: coreMode === 'A' }"
        :disabled="switchLoading.switch3"
        type="button"
        @click="$emit('set-core-mode', 'A')"
      >
        <strong>兼容模式</strong>
        <span>表外域名优先国内 DNS 解析，保证速度。</span>
      </button>
      <button
        class="system-core-mode-option"
        :class="{ active: coreMode === 'B' }"
        :disabled="switchLoading.switch3"
        type="button"
        @click="$emit('set-core-mode', 'B')"
      >
        <strong>安全模式</strong>
        <span>表外域名仅用国外 DNS 解析，阻止 DNS 泄漏。</span>
      </button>
    </div>

    <div v-else class="core-mode-hints">
      <p class="muted">兼容模式：表外域名优先国内dns解析，保证速度。</p>
      <p class="muted">安全模式：表外域名仅用国外dns解析，阻止dns泄漏。</p>
    </div>
  </section>
</template>

<style scoped>
.system-core-mode-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 10px;
}

.system-core-mode-option {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 6px;
  min-width: 0;
  padding: 14px 16px;
  border: 1px solid var(--line);
  border-radius: 16px;
  background: rgba(var(--panel-glass-rgb), var(--panel-glass-opacity));
  color: inherit;
  text-align: left;
  cursor: pointer;
  transition: 0.18s ease;
  backdrop-filter: blur(var(--panel-glass-blur));
  -webkit-backdrop-filter: blur(var(--panel-glass-blur));
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.24);
}

.system-core-mode-option strong {
  font-size: 1rem;
  line-height: 1.2;
}

.system-core-mode-option span {
  color: var(--ink-1);
  font-size: 0.82rem;
  line-height: 1.5;
}

.system-core-mode-option:hover:not(:disabled) {
  border-color: rgba(var(--brand-rgb), 0.28);
}

.system-core-mode-option.active {
  border-color: rgba(var(--brand-rgb), 0.34);
  background: linear-gradient(135deg, rgba(var(--brand-rgb), 0.14), rgba(var(--panel-glass-rgb), var(--panel-glass-opacity)));
  box-shadow: 0 10px 18px rgba(var(--brand-rgb), 0.08);
}

.system-core-mode-option:disabled {
  opacity: 0.64;
  cursor: default;
}

@media (max-width: 900px) {
  .system-core-mode-grid {
    grid-template-columns: 1fr;
  }
}
</style>
