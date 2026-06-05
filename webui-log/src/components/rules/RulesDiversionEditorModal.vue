<script setup>
defineProps({
  editor: {
    type: Object,
    required: true
  },
  diversionTypeOptions: {
    type: Array,
    default: () => []
  },
  isEditing: {
    type: Boolean,
    default: false
  }
})

defineEmits([
  'close',
  'save',
  'apply-autofill',
  'type-change',
  'url-input',
  'name-input',
  'files-input'
])
</script>

<template>
  <div class="modal-mask">
    <section class="panel form-modal-card">
      <header class="panel-header">
        <h3>{{ isEditing ? '编辑分流规则' : '新增分流规则' }}</h3>
        <button class="btn tiny secondary" type="button" @click="$emit('close')">✕</button>
      </header>
      <div class="form-grid">
        <label>类型</label>
        <select v-model="editor.type" @change="$emit('type-change')">
          <option v-for="item in diversionTypeOptions" :key="item.value" :value="item.value">{{ item.label }}</option>
        </select>
        <label v-if="!isEditing">自动识别</label>
        <div v-if="!isEditing" class="autofill-actions">
          <small class="muted">输入 URL 后会自动识别名称和本地文件路径，也可以手动点击“自动识别”。</small>
          <button class="btn tiny secondary" type="button" @click="$emit('apply-autofill')">自动识别</button>
        </div>
        <label>URL</label>
        <input v-model="editor.url" @input="$emit('url-input')" />
        <label>名称</label>
        <input v-model="editor.name" @input="$emit('name-input')" />
        <label>本地文件</label>
        <input v-model="editor.files" placeholder="例如 /cus/mosdns/srs/geo/cn.json" @input="$emit('files-input')" />
        <label>更新间隔 (小时)</label>
        <input v-model.number="editor.update_interval_hours" type="number" min="1" />
        <label>启用</label>
        <label class="switch-inline"><input v-model="editor.enabled" type="checkbox" /><span>{{ editor.enabled ? '已启用' : '已禁用' }}</span></label>
        <label>自动更新</label>
        <label class="switch-inline"><input v-model="editor.auto_update" type="checkbox" /><span>{{ editor.auto_update ? '开启' : '关闭' }}</span></label>
        <label>启用正则</label>
        <label class="switch-inline"><input v-model="editor.enable_regexp" type="checkbox" /><span>{{ editor.enable_regexp ? '开启' : '关闭' }}</span></label>
      </div>
      <div class="actions">
        <button class="btn secondary" @click="$emit('close')">取消</button>
        <button class="btn primary" @click="$emit('save')">保存</button>
      </div>
    </section>
  </div>
</template>
