<script setup>
defineProps({
  coreMode: {
    type: String,
    default: "",
  },
  switchLoading: {
    type: Object,
    required: true,
  },
});

defineEmits(["set-core-mode"]);
</script>

<template>
  <section class="panel control-module system-grid-dual-item system-mode-panel">
    <div class="system-mode-head">
      <h3 class="system-mode-title">
        <span>核心运行模式</span>
      </h3>

      <div class="system-mode-actions">
        <button
          class="btn tiny system-mode-btn"
          :class="coreMode === 'A' ? 'primary is-active' : 'secondary'"
          :disabled="switchLoading.switch3"
          @click="$emit('set-core-mode', 'A')"
        >
          兼容模式
        </button>
        <button
          class="btn tiny system-mode-btn"
          :class="coreMode === 'B' ? 'primary is-active' : 'secondary'"
          :disabled="switchLoading.switch3"
          @click="$emit('set-core-mode', 'B')"
        >
          安全模式
        </button>
      </div>
    </div>

    <div class="system-mode-notes">
      <p :class="{ active: coreMode === 'A' }">
        <strong>兼容：</strong>表外域名国内解析，保证速度
      </p>
      <p :class="{ active: coreMode === 'B' }">
        <strong>安全：</strong>表外域名国外解析，阻止泄漏
      </p>
    </div>
  </section>
</template>

<style scoped>
.system-mode-panel {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 12px 14px;
  container-type: inline-size;
}

.system-mode-head {
  display: grid;
  grid-template-columns: max-content minmax(0, 1fr);
  gap: 8px;
  align-items: center;
}

.system-mode-title {
  display: flex;
  margin: 0;
  color: var(--ink-0);
  font-size: 1rem;
  font-weight: 800;
  line-height: 1.05;
  text-align: left;
}

.system-mode-title span {
  white-space: nowrap;
}

.system-mode-actions {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 8px;
}

.system-mode-btn {
  width: 100%;
  min-width: 0;
  min-height: 40px;
  padding-inline: 4px;
  border-radius: 12px;
  font-size: 0.78rem;
  font-weight: 800;
  line-height: 1.15;
  white-space: nowrap;
  text-align: center;
}

.btn.system-mode-btn.is-active {
  border: 2px solid var(--system-mode-active-ring, #111827) !important;
  color: var(--system-mode-active-text, #111827) !important;
  box-shadow:
    inset 0 0 0 2px var(--system-mode-active-ring, #111827),
    0 0 0 1px var(--system-mode-active-ring, #111827) !important;
}

.system-mode-notes {
  display: grid;
  gap: 5px;
}

.system-mode-notes p {
  margin: 0;
  color: var(--ink-1);
  font-size: 0.79rem;
  line-height: 1.3;
}

.system-mode-notes p.active {
  color: var(--ink-1);
}

.system-mode-notes strong {
  color: inherit;
  font-weight: 700;
}

@container (max-width: 260px) {
  .system-mode-head {
    grid-template-columns: 1fr;
    align-items: stretch;
  }

  .system-mode-actions {
    grid-template-columns: 1fr;
  }
}

@media (max-width: 640px) {
  .system-mode-head {
    grid-template-columns: 1fr;
    align-items: stretch;
  }

  .system-mode-actions {
    grid-template-columns: 1fr;
  }
}
</style>
