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

defineEmits(['load-overrides', 'add-replacement', 'save-overrides', 'remove-replacement'])
</script>

<template>
  <section class="panel control-module control-module-wide">
    <header class="module-head">
      <div>
        <h3>高级替换规则</h3>
        <p class="muted">可配置 DNS 覆盖映射。修改后点击保存并应用。</p>
      </div>
      <div class="actions">
        <button class="btn tiny secondary" @click="$emit('load-overrides')">读取当前</button>
        <button class="btn tiny secondary" @click="$emit('add-replacement')">添加规则</button>
        <button class="btn tiny primary" :disabled="applyingOverrides" @click="$emit('save-overrides')">
          {{ applyingOverrides ? '保存中...' : '保存并应用' }}
        </button>
      </div>
    </header>

    <div class="table-wrap replacements-table-wrap">
      <table class="replacements-table">
        <thead>
          <tr>
            <th>状态</th>
            <th>原值</th>
            <th>新值</th>
            <th>备注</th>
            <th>操作</th>
          </tr>
        </thead>
        <tbody>
          <tr v-if="overrides.replacements.length === 0">
            <td colspan="5" class="empty">暂无规则</td>
          </tr>
          <tr v-for="(item, index) in overrides.replacements" :key="`rep-${index}`">
            <td>{{ item.result || '未保存' }}</td>
            <td><input v-model="item.original" placeholder="例如: 1.1.1.1" /></td>
            <td><input v-model="item.new" placeholder="例如: 127.0.0.1" /></td>
            <td><input v-model="item.comment" placeholder="可选备注" /></td>
            <td><button class="btn tiny danger" @click="$emit('remove-replacement', index)">删除</button></td>
          </tr>
        </tbody>
      </table>
    </div>
  </section>
</template>
