<script setup>
defineProps({
  routingMode: {
    type: String,
    required: true,
  },
  switchLoading: {
    type: Object,
    required: true,
  },
});

defineEmits(["set-dns-routing-mode"]);
</script>

<template>
  <section class="panel control-module system-grid-dual-item system-mode-panel">
    <div class="system-mode-head">
      <h3 class="system-mode-title">
        <span>DNS 分流模式</span>
      </h3>

      <div class="system-mode-actions">
        <button
          class="btn tiny system-mode-btn"
          :class="routingMode === 'A' ? 'primary is-active' : 'secondary'"
          :disabled="switchLoading.switch17"
          @click="$emit('set-dns-routing-mode', 'A')"
        >
          FakeIP
        </button>
        <button
          class="btn tiny system-mode-btn"
          :class="routingMode === 'B' ? 'primary is-active' : 'secondary'"
          :disabled="switchLoading.switch17"
          @click="$emit('set-dns-routing-mode', 'B')"
        >
          Redir-Host
        </button>
      </div>
    </div>

    <div class="system-mode-notes">
      <p :class="{ active: routingMode === 'A' }">
        <strong>FakeIP：</strong>国外域名返回 FakeIP
      </p>
      <p :class="{ active: routingMode === 'B' }">
        <strong>Redir-Host：</strong>国外域名返回真实 IP
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
  display: inline-flex;
  align-items: center;
  justify-content: center;
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
  font-family: Arial, Helvetica, sans-serif;
  font-variant-ligatures: none;
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
