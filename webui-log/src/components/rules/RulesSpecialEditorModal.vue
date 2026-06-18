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
          <h3>{{ editor.slot ? '编辑专属分流组' : '新增专属分流组' }}</h3>
          <button class="btn tiny secondary" type="button" @click="$emit('close')">✕</button>
        </header>
        <div class="form-grid">
          <label>槽位 (&gt;=50，留空自动分配)</label>
          <input v-model.number="editor.slot" type="number" min="0" />
          <label>名称</label>
          <input v-model="editor.name" placeholder="例如 移动上游" />
          <label>监听端口</label>
          <input v-model="editor.listenPort" type="number" min="1" max="65535" placeholder="留空则沿用原逻辑" />
          <label>仅自定义端口生效</label>
          <label class="switch-inline">
            <input v-model="editor.customPortOnly" type="checkbox" :disabled="!String(editor.listenPort || '').trim()" />
          </label>
        </div>
        <p class="muted">1.未勾选则53端口及自定义端口均生效</p>
        <p class="muted">2.保存后会同步更新专属分流组在 53 主链和自定义端口上的生效方式。</p>
        <div class="actions">
          <button class="btn secondary" @click="$emit('close')">取消</button>
          <button class="btn primary" @click="$emit('save')">保存</button>
        </div>
      </section>
    </div>
  </Teleport>
</template>
