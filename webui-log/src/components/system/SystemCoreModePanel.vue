<script setup>
defineProps({
  coreMode: {
    type: String,
    default: ''
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
      <div class="actions">
        <button class="btn tiny core-mode-btn" :class="coreMode === 'A' ? 'primary is-active' : 'secondary'" :disabled="switchLoading.switch3" @click="$emit('set-core-mode', 'A')">兼容模式</button>
        <button class="btn tiny core-mode-btn" :class="coreMode === 'B' ? 'primary is-active' : 'secondary'" :disabled="switchLoading.switch3" @click="$emit('set-core-mode', 'B')">安全模式</button>
      </div>
    </header>
    <div class="core-mode-hints">
      <p class="muted">兼容模式：表外域名优先国内dns解析，保证速度。</p>
      <p class="muted">安全模式：表外域名仅用国外dns解析，阻止dns泄漏。</p>
    </div>
  </section>
</template>
