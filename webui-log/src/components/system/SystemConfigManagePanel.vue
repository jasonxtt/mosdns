<script setup>
defineProps({
  configManaging: {
    type: Object,
    required: true
  },
  configVersion: {
    type: Object,
    default: () => ({})
  },
  configManagementEnabled: {
    type: Boolean,
    default: true
  }
})

defineEmits(['save-settings', 'backup-config', 'apply-remote-config'])
</script>

<template>
  <section class="panel control-module control-module--mini">
    <h3>配置管理</h3>
    <template v-if="configManagementEnabled">
      <div class="module-kv-list">
        <div class="control-line">
          <strong>当前配置版本</strong>
          <span>{{ configVersion.versionText || '--' }} <span v-if="configVersion.statusText" class="mini-badge">{{ configVersion.statusText }}</span></span>
        </div>
      </div>
      <div class="module-form-stack">
        <label class="mini-field">
          <span>本地目录</span>
          <input v-model="configManaging.localDir" placeholder="/cus/mosdns" @change="$emit('save-settings')" />
        </label>
        <label class="mini-field">
          <span>远程 ZIP URL</span>
          <input v-model="configManaging.remoteUrl" placeholder="https://example.com/mosdns.zip" @change="$emit('save-settings')" />
        </label>
      </div>
      <div class="actions">
        <button class="btn tiny secondary" :disabled="configManaging.backingUp" @click="$emit('backup-config')">
          {{ configManaging.backingUp ? '备份中...' : '备份配置' }}
        </button>
        <button class="btn tiny primary" :disabled="configManaging.updating" @click="$emit('apply-remote-config')">
          {{ configManaging.updating ? '更新中...' : '应用远程配置' }}
        </button>
      </div>
    </template>
    <p v-else class="muted">OpenWrt版本不支持使用配置管理远程更新配置。</p>
  </section>
</template>
