<script setup>
defineProps({
  configManaging: {
    type: Object,
    required: true
  },
  configManageSupported: {
    type: Boolean,
    default: true
  },
  configManageMessage: {
    type: String,
    default: ''
  },
  configVersion: {
    type: Object,
    default: () => ({})
  }
})

defineEmits(['save-settings', 'backup-config', 'apply-remote-config'])
</script>

<template>
  <section class="panel control-module control-module--mini">
    <h3>配置管理</h3>
    <div class="module-kv-list">
      <div class="control-line">
        <strong>当前配置版本</strong>
        <span>{{ configVersion.versionText || '--' }} <span v-if="configVersion.statusText" class="mini-badge">{{ configVersion.statusText }}</span></span>
      </div>
    </div>
    <div class="module-form-stack">
      <label class="mini-field">
        <span>本地目录</span>
        <input v-model="configManaging.localDir" :disabled="!configManageSupported" placeholder="/cus/mosdns" @change="$emit('save-settings')" />
      </label>
      <label class="mini-field">
        <span>远程 ZIP URL</span>
        <input v-model="configManaging.remoteUrl" :disabled="!configManageSupported" placeholder="https://example.com/mosdns.zip" @change="$emit('save-settings')" />
      </label>
    </div>
    <p v-if="configManageMessage" class="muted">{{ configManageMessage }}</p>
    <div class="actions">
      <button class="btn tiny secondary" :disabled="configManaging.backingUp || !configManageSupported" @click="$emit('backup-config')">
        {{ configManaging.backingUp ? '备份中...' : '备份配置' }}
      </button>
      <button class="btn tiny primary" :disabled="configManaging.updating || !configManageSupported" @click="$emit('apply-remote-config')">
        {{ configManaging.updating ? '更新中...' : '应用远程配置' }}
      </button>
    </div>
  </section>
</template>
