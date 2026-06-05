<script setup>
defineProps({
  webuiPort: {
    type: Object,
    required: true
  }
})

defineEmits(['apply-port'])
</script>

<template>
  <section class="panel control-module webui-port-module system-grid-dual-item">
    <h3>WebUI 端口</h3>
    <div class="webui-port-inline-row">
      <label class="webui-port-inline-field">
        <span>当前端口</span>
        <input
          :value="webuiPort.loading ? '读取中...' : String(webuiPort.activePort || '--')"
          type="text"
          readonly
        />
      </label>
      <label class="webui-port-inline-field">
        <span>目标端口</span>
        <input v-model="webuiPort.input" type="number" min="1" max="65535" placeholder="例如 9099" />
      </label>
    </div>
    <div class="actions webui-port-actions">
      <button class="btn tiny primary" :disabled="webuiPort.saving || webuiPort.loading" @click="$emit('apply-port')">
        {{ webuiPort.saving ? '处理中...' : '确认并重启 MosDNS' }}
      </button>
    </div>
  </section>
</template>
