<script setup>
defineProps({
  audit: {
    type: Object,
    required: true
  }
})

defineEmits(['submit-capacity'])
</script>

<template>
  <section class="panel control-module control-module--mini">
    <h3>详细日志热数据上限</h3>
    <div class="control-line">
      <strong>当前上限</strong>
      <span>{{ audit.capacity === null ? '读取中...' : Number(audit.capacity).toLocaleString() }}</span>
    </div>
    <form class="capacity-form" @submit.prevent="$emit('submit-capacity')">
      <input v-model="audit.newCapacity" :disabled="audit.busy || audit.capacity === null" type="number" min="1" max="400000" placeholder="输入热日志上限" />
      <button class="btn tiny primary" :disabled="audit.busy || audit.capacity === null" type="submit">{{ audit.busy ? '处理中...' : '设置' }}</button>
    </form>
    <p class="muted">仅影响近期详细日志热数据保留条数；1小时到7天统计按时间窗单独汇总，设置新上限会清空当前详细日志。</p>
  </section>
</template>
