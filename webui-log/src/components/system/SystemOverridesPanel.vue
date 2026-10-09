<script setup>
import { isModified, NATIVE_OVERRIDES_NOTE } from '../../utils/nativeSettings'

defineProps({
  applyingOverrides: {
    type: Boolean,
    default: false
  },
  overrides: {
    type: Object,
    required: true
  },
  // Native runtime: changes apply at once and each value can return to the
  // configuration file's value (`sources` is {socks5, ecs} of 'yaml' | 'settings').
  native: {
    type: Boolean,
    default: false
  },
  sources: {
    type: Object,
    default: () => ({})
  }
})

defineEmits(['load-overrides', 'save-overrides', 'restore-override'])
</script>

<template>
  <section class="panel control-module control-module--mini">
    <h3>SOCKS5 / ECS 覆盖</h3>
    <p v-if="native" class="muted override-note" data-native-overrides-note>{{ NATIVE_OVERRIDES_NOTE }}</p>
    <div class="module-form-stack">
      <label class="mini-field">
        <span>socks5 代理</span>
        <input v-model="overrides.socks5" placeholder="host:port" />
        <span v-if="native && isModified(sources.socks5)" class="override-mark" data-override-modified="socks5">
          已修改
          <button class="btn tiny secondary" type="button" :disabled="applyingOverrides" @click="$emit('restore-override', 'socks5')">恢复默认</button>
        </span>
      </label>
      <label class="mini-field">
        <span>ECS IP</span>
        <input v-model="overrides.ecs" placeholder="IPv4 / IPv6" />
        <span v-if="native && isModified(sources.ecs)" class="override-mark" data-override-modified="ecs">
          已修改
          <button class="btn tiny secondary" type="button" :disabled="applyingOverrides" @click="$emit('restore-override', 'ecs')">恢复默认</button>
        </span>
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
