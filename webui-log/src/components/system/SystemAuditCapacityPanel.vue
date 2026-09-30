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
      <input v-model="audit.newCapacity" :disabled="audit.busy || audit.capacity === null" type="number" min="0" max="400000" step="1" placeholder="输入热日志上限（0 表示不保留）" />
      <button class="btn tiny primary" :disabled="audit.busy || audit.capacity === null" type="submit">{{ audit.busy ? '处理中...' : '设置' }}</button>
    </form>
    <p class="muted">0 到 400000 条；0 表示不保留详细日志。设置新上限会清空 retained 审计日志，因此 v2 统计、时间窗和日志列表会随当前 retained ring 变化。</p>
  </section>
</template>
