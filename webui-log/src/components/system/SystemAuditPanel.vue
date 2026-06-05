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
    <div class="button-group-vue">
      <button class="btn tiny primary" @click="$emit('toggle-audit')">{{ audit.capturing ? '停止审计' : '启动审计' }}</button>
      <button class="btn tiny danger" @click="$emit('clear-logs')">清空日志</button>
    </div>
  </section>
</template>
