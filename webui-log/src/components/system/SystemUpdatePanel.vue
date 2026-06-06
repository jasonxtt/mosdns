<script setup>
defineProps({
  hasUpdate: {
    type: Boolean,
    default: false
  },
  showV3Callout: {
    type: Boolean,
    default: false
  },
  update: {
    type: Object,
    required: true
  },
  updateBannerText: {
    type: String,
    default: ''
  },
  updateLastCheckedText: {
    type: String,
    default: '--'
  },
  updateLatestBadge: {
    type: Boolean,
    default: false
  }
})

defineEmits(['check-update', 'apply-update', 'apply-force-update', 'apply-v3-update'])
</script>

<template>
  <section class="panel control-module control-module--mini">
    <h3>版本与更新</h3>
    <div class="module-kv-list">
      <div class="control-line"><strong>当前版本</strong><span>{{ update.status?.current_version || '未知' }}</span></div>
      <div class="control-line"><strong>最新版本</strong><span>{{ update.status?.latest_version || '--' }} <span v-if="updateLatestBadge" class="mini-badge">已是最新</span></span></div>
      <div class="control-line"><strong>上次检查</strong><span>{{ updateLastCheckedText }}</span></div>
      <div class="control-line">
        <strong>配置结构</strong>
        <span>{{ update.status?.config_schema_applied ?? 0 }} / {{ update.status?.config_schema_required ?? 0 }}</span>
      </div>
      <div v-if="update.status?.config_update_status" class="control-line">
        <strong>配置升级</strong>
        <span>{{ ({ success: '成功', failed: '失败', in_progress: '进行中' })[update.status.config_update_status] || update.status.config_update_status }}</span>
      </div>
    </div>
    <p class="update-banner">{{ updateBannerText }}</p>
    <p v-if="update.status?.config_update_message" class="update-banner">
      {{ update.status.config_update_message }}
      <span v-if="update.status?.config_update_error">：{{ update.status.config_update_error }}</span>
    </p>
    <div class="actions">
      <button class="btn tiny secondary" :disabled="update.loading" @click="$emit('check-update')">{{ update.loading ? '处理中...' : '检查更新' }}</button>
      <button class="btn tiny primary" :disabled="update.loading || !hasUpdate" @click="$emit('apply-update')">立即更新</button>
      <button class="btn tiny danger" :disabled="update.loading || !update.status?.download_url" @click="$emit('apply-force-update')">强制更新</button>
      <button v-if="showV3Callout" class="btn tiny warning" :disabled="update.loading" @click="$emit('apply-v3-update')">切换 v3</button>
    </div>
  </section>
</template>
