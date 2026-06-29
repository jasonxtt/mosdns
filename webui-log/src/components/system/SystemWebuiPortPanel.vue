<script setup>
defineProps({
  webuiPort: {
    type: Object,
    required: true,
  },
  changeSupported: {
    type: Boolean,
    default: true,
  },
});

defineEmits(["apply-port"]);
</script>

<template>
  <section class="panel control-module webui-port-module system-grid-dual-item">
    <div class="webui-port-layout">
      <h3 class="webui-port-title">
        <span>WebUI</span>
        <span>端口设置</span>
      </h3>

      <input
        v-model="webuiPort.input"
        :disabled="!changeSupported"
        type="number"
        min="1"
        max="65535"
        placeholder="目标端口"
      />

      <button
        class="btn tiny primary webui-port-btn"
        :disabled="webuiPort.saving || webuiPort.loading || !changeSupported"
        @click="$emit('apply-port')"
      >
        {{ webuiPort.saving ? "处理中..." : "保存" }}
      </button>
    </div>
    <p v-if="webuiPort.message" class="muted webui-port-message">
      {{ webuiPort.message }}
    </p>
  </section>
</template>

<style scoped>
.webui-port-module {
  display: flex;
  flex: 1 1 auto;
  flex-direction: column;
  gap: 8px;
  min-height: 0;
  padding: 12px 14px;
  container-type: inline-size;
}

.webui-port-layout {
  display: grid;
  grid-template-columns: 88px minmax(0, 1fr) minmax(0, 1fr);
  gap: 8px;
  width: 100%;
  height: 100%;
  align-items: center;
}

.webui-port-title {
  display: flex;
  flex-direction: column;
  justify-content: center;
  gap: 2px;
  height: 100%;
  margin: 0;
  color: var(--ink-0);
  font-size: 0.96rem;
  font-weight: 800;
  line-height: 1.08;
  text-align: left;
}

.webui-port-title span {
  white-space: nowrap;
}

.webui-port-title span + span {
  align-self: flex-start;
}

.webui-port-layout input,
.webui-port-btn {
  box-sizing: border-box;
  width: 100%;
  height: 40px;
  min-height: 40px;
}

.webui-port-layout input {
  min-width: 0;
  padding: 7px 12px;
  font-size: 0.92rem;
  font-variant-numeric: tabular-nums;
  text-align: center;
}

.webui-port-layout input:disabled {
  cursor: not-allowed;
  opacity: 0.62;
}

.webui-port-layout input::-webkit-outer-spin-button,
.webui-port-layout input::-webkit-inner-spin-button {
  margin: 0;
  -webkit-appearance: none;
}

.webui-port-layout input[type="number"] {
  -moz-appearance: textfield;
}

.webui-port-btn {
  min-width: 0;
  padding-inline: 12px;
  font-size: 0.76rem;
  font-weight: 800;
  white-space: nowrap;
}

.webui-port-message {
  margin: 0;
  font-size: 0.76rem;
  line-height: 1.35;
}

@container (max-width: 240px) {
  .webui-port-layout {
    grid-template-columns: 88px minmax(0, 1fr);
  }

  .webui-port-btn {
    grid-column: 2;
  }
}

@container (max-width: 200px) {
  .webui-port-layout {
    grid-template-columns: 1fr;
    align-items: stretch;
  }

  .webui-port-title {
    flex-direction: row;
    justify-content: flex-start;
    gap: 0.25em;
    height: auto;
  }

  .webui-port-title span + span {
    align-self: auto;
  }

  .webui-port-btn {
    grid-column: auto;
  }
}

@media (max-width: 640px) {
  .webui-port-layout {
    grid-template-columns: 1fr;
    align-items: stretch;
  }

  .webui-port-title {
    flex-direction: row;
    justify-content: flex-start;
    gap: 0.25em;
    height: auto;
  }

  .webui-port-title span + span {
    align-self: auto;
  }

  .webui-port-btn {
    grid-column: auto;
  }
}
</style>
