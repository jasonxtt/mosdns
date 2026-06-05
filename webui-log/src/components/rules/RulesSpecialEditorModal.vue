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
  <div class="modal-mask">
    <section class="panel form-modal-card">
      <header class="panel-header">
        <h3>{{ editor.slot ? '修改专属分流组' : '新增专属分流组' }}</h3>
        <button class="btn tiny secondary" type="button" @click="$emit('close')">✕</button>
      </header>
      <div class="form-grid">
        <label>槽位 (&gt;=50，留空自动分配)</label>
        <input v-model.number="editor.slot" type="number" min="0" />
        <label>名称</label>
        <input v-model="editor.name" placeholder="例如 移动上游" />
      </div>
      <div class="actions">
        <button class="btn secondary" @click="$emit('close')">取消</button>
        <button class="btn primary" @click="$emit('save')">保存</button>
      </div>
    </section>
  </div>
</template>
