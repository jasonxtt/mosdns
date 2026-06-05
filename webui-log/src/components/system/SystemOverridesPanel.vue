<script setup>
defineProps({
  applyingOverrides: {
    type: Boolean,
    default: false
  },
  overrides: {
    type: Object,
    required: true
  }
})

defineEmits(['load-overrides', 'save-overrides'])
</script>

<template>
  <section class="panel control-module control-module--mini">
    <h3>SOCKS5 / ECS 覆盖</h3>
    <div class="module-form-stack">
      <label class="mini-field">
        <span>socks5 代理</span>
        <input v-model="overrides.socks5" placeholder="host:port" />
      </label>
      <label class="mini-field">
        <span>ECS IP</span>
        <input v-model="overrides.ecs" placeholder="IPv4 / IPv6" />
      </label>
    </div>
    <div class="actions">
      <button class="btn tiny secondary" @click="$emit('load-overrides')">读取当前</button>
      <button class="btn tiny primary" :disabled="applyingOverrides" @click="$emit('save-overrides')">
        {{ applyingOverrides ? '保存中...' : '保存并应用' }}
      </button>
    </div>
  </section>
</template>
