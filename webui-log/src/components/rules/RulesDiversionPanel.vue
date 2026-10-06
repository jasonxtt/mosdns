<script setup>
defineProps({
  loading: {
    type: Boolean,
    default: false
  },
  diversionRules: {
    type: Array,
    default: () => []
  },
  formatTime: {
    type: Function,
    required: true
  },
  nativeLocalText: {
    type: Boolean,
    default: false
  },
  canMutate: {
    type: Boolean,
    default: true
  }
})

defineEmits(['create', 'update-all', 'toggle', 'update', 'edit', 'delete'])
</script>

<template>
  <section class="sub-panel">
    <div class="actions">
      <button class="btn primary entry-action-btn" :disabled="!canMutate" @click="$emit('create')">新增分流规则</button>
      <button
        class="btn warning entry-action-btn"
        :disabled="nativeLocalText || !canMutate"
        :title="nativeLocalText ? '原生运行时不支持在线下载或更新' : ''"
        @click="$emit('update-all')"
      >更新全部规则</button>
    </div>
    <p v-if="nativeLocalText" class="muted" role="note">
      原生模式仅支持本地 .txt 规则；不支持下载、自动更新、正则或高级数据格式。带有不支持配置的已有记录会保留并显示为只读。
    </p>
    <div class="table-wrap adaptive-table-wrap rules-diversion-wrap">
      <table class="rules-adaptive-table rules-diversion-table">
        <thead>
          <tr>
            <th>启用</th>
            <th>类型</th>
            <th>名称</th>
            <th>文件</th>
            <th>URL</th>
            <th>规则数</th>
            <th>更新时间</th>
            <th>操作</th>
          </tr>
        </thead>
        <tbody>
          <tr v-if="loading">
            <td colspan="8" class="empty">加载中...</td>
          </tr>
          <tr v-else-if="diversionRules.length === 0">
            <td colspan="8" class="empty">{{ nativeLocalText ? '暂无本地分流规则' : '暂无在线分流规则' }}</td>
          </tr>
          <tr v-for="rule in diversionRules" :key="`${rule.type}:${rule.name}`" :class="{ disabled: !rule.enabled || rule.__readOnly }">
            <td>
              <label class="switch switch-table">
                <input type="checkbox" :checked="Boolean(rule.enabled)" :disabled="rule.__readOnly || !canMutate" @change="$emit('toggle', rule)" />
                <span class="slider"></span>
              </label>
            </td>
            <td :title="rule.__typeLabel">{{ rule.__typeLabel }}</td>
            <td :title="rule.__unavailableReason || rule.name">
              {{ rule.name }}
              <span v-if="rule.__readOnly" class="muted" :title="rule.__unavailableReason">（原生只读）</span>
            </td>
            <td class="mono" :title="rule.files">{{ rule.files }}</td>
            <td class="mono" :title="rule.url">{{ rule.url }}</td>
            <td class="text-right">{{ Number(rule.rule_count || 0).toLocaleString() }}</td>
            <td class="mono" :title="formatTime(rule.last_updated)">{{ formatTime(rule.last_updated) }}</td>
            <td class="row-actions">
              <button class="btn tiny warning" :disabled="nativeLocalText" :title="nativeLocalText ? '原生运行时不支持在线更新' : ''" @click="$emit('update', rule)">更新</button>
              <button class="btn tiny secondary" :disabled="rule.__readOnly || !canMutate" @click="$emit('edit', rule)">编辑</button>
              <button class="btn tiny danger" :disabled="rule.__readOnly || !canMutate" @click="$emit('delete', rule)">删除</button>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  </section>
</template>
