<script setup>
defineProps({
  autoRefresh: {
    type: Object,
    required: true
  }
})

defineEmits(['toggle', 'interval-change'])
</script>

<template>
  <section class="panel control-module control-module--mini">
    <h3>自动刷新</h3>
    <div class="control-line">
      <strong>启用状态</strong>
      <label class="switch switch-table">
        <input type="checkbox" :checked="autoRefresh.enabled" @change="$emit('toggle', $event)" />
        <span class="slider"></span>
      </label>
    </div>
    <div class="control-line">
      <strong>刷新间隔</strong>
      <div class="capacity-form" style="max-width: 170px;">
        <input v-model.number="autoRefresh.intervalSeconds" type="number" min="5" @change="$emit('interval-change')" />
        <span class="muted">秒</span>
      </div>
    </div>
  </section>
</template>
