<script setup>
defineProps({
  loading: {
    type: Boolean,
    default: false
  },
  specialGroups: {
    type: Array,
    default: () => []
  }
})

defineEmits(['create', 'edit', 'delete'])
</script>

<template>
  <section class="sub-panel">
    <div class="actions">
      <button class="btn primary entry-action-btn" @click="$emit('create')">新增专属分流组</button>
    </div>
    <div class="table-wrap">
      <table>
        <thead>
          <tr>
            <th>槽位</th>
            <th>名称</th>
            <th>上游组 Tag</th>
            <th>分流插件 Tag</th>
            <th>操作</th>
          </tr>
        </thead>
        <tbody>
          <tr v-if="loading">
            <td colspan="5" class="empty">加载中...</td>
          </tr>
          <tr v-else-if="specialGroups.length === 0">
            <td colspan="5" class="empty">暂无专属分流组</td>
          </tr>
          <tr v-for="group in specialGroups" :key="group.slot">
            <td>{{ group.slot }}</td>
            <td>{{ group.name }}</td>
            <td class="mono">{{ group.upstream_plugin_tag }}</td>
            <td class="mono">{{ group.diversion_plugin_tag }}</td>
            <td class="row-actions">
              <button class="btn tiny secondary" @click="$emit('edit', group)">改名</button>
              <button class="btn tiny danger" @click="$emit('delete', group)">删除</button>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  </section>
</template>
