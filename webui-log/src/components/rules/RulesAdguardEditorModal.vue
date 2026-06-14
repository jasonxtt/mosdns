<script setup>
defineProps({
  editor: {
    type: Object,
    required: true
  }
})

defineEmits(['close', 'save'])
</script>

<template>
  <Teleport to="body">
    <div class="modal-mask" @click.self="$emit('close')">
      <section class="panel form-modal-card">
        <header class="panel-header">
          <h3>{{ editor.id ? '编辑 AdGuard 规则' : '新增 AdGuard 规则' }}</h3>
          <button class="btn tiny secondary" type="button" @click="$emit('close')">✕</button>
        </header>
        <div class="form-grid">
          <label>名称</label>
          <input v-model="editor.name" />
          <label>URL</label>
          <input v-model="editor.url" />
          <label>更新间隔 (小时)</label>
          <input v-model.number="editor.update_interval_hours" type="number" min="1" />
          <label>启用</label>
          <label class="switch-inline"><input v-model="editor.enabled" type="checkbox" /><span>{{ editor.enabled ? '已启用' : '已禁用' }}</span></label>
          <label>自动更新</label>
          <label class="switch-inline"><input v-model="editor.auto_update" type="checkbox" /><span>{{ editor.auto_update ? '开启' : '关闭' }}</span></label>
        </div>
        <div class="actions">
          <button class="btn secondary" @click="$emit('close')">取消</button>
          <button class="btn primary" @click="$emit('save')">保存</button>
        </div>
      </section>
    </div>
  </Teleport>
</template>
