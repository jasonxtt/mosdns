<script setup>
defineProps({
  audit: {
    type: Object,
    required: true
  }
})

defineEmits(['toggle-audit', 'clear-logs'])
</script>

<template>
  <section class="panel control-module control-module--mini">
    <h3>审计控制</h3>
    <div class="control-line">
      <strong>运行状态</strong>
      <span>{{ audit.capturing === null ? '读取中...' : (audit.capturing ? '运行中' : '已停止') }}</span>
    </div>
    <p v-if="audit.error" class="muted audit-panel-error">{{ audit.error }}</p>
    <div class="button-group-vue">
      <button class="btn tiny primary" :disabled="audit.busy || audit.capturing === null" @click="$emit('toggle-audit')">{{ audit.busy ? '处理中...' : (audit.capturing ? '停止审计' : '启动审计') }}</button>
      <button class="btn tiny danger" :disabled="audit.busy || audit.capturing === null" @click="$emit('clear-logs')">清空日志</button>
    </div>
  </section>
</template>

<style scoped>
.audit-panel-error {
  color: var(--warn);
  font-size: 0.72rem;
  margin: 6px 0 0;
}
</style>
