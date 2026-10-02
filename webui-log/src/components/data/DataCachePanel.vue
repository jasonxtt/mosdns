<script setup>
defineProps({
  cacheClearingAll: {
    type: Boolean,
    default: false
  },
  cacheClearingByTag: {
    type: Object,
    required: true
  },
  cacheError: {
    type: String,
    default: ''
  },
  cacheRows: {
    type: Array,
    default: () => []
  }
})

defineEmits(['clear-all', 'open-cache', 'clear-cache'])

function metricValue(value) {
  return value === null || value === undefined ? '—' : Number(value).toLocaleString()
}
</script>

<template>
  <section class="panel sub-panel data-module cache-module">
    <header class="panel-header cache-module-head">
      <div>
        <h3>缓存管理</h3>
      </div>
      <div class="actions">
        <button class="btn danger cache-clear-btn" :disabled="cacheClearingAll" @click="$emit('clear-all')">
          {{ cacheClearingAll ? '清空中...' : '清空所有缓存' }}
        </button>
      </div>
    </header>

    <p v-if="cacheError" role="alert">缓存列表加载失败：{{ cacheError }}</p>

    <div class="table-wrap cache-table-wrap data-scroll-wrap">
      <table class="cache-adaptive-table">
        <thead>
          <tr>
            <th>缓存名称</th>
            <th>请求总数</th>
            <th>缓存命中</th>
            <th>过期命中</th>
            <th>命中率</th>
            <th>过期命中率</th>
            <th>条目数</th>
            <th>操作</th>
          </tr>
        </thead>
        <tbody>
          <tr v-if="cacheRows.length === 0">
            <td colspan="8" class="empty">{{ cacheError ? '缓存数据加载失败' : '暂无缓存数据' }}</td>
          </tr>
          <tr v-for="cache in cacheRows" :key="cache.key">
            <td>{{ cache.name }}</td>
            <td>{{ metricValue(cache.query_total) }}</td>
            <td>{{ metricValue(cache.hit_total) }}</td>
            <td>{{ metricValue(cache.lazy_hit_total) }}</td>
            <td>{{ cache.hit_rate }}</td>
            <td>{{ cache.lazy_hit_rate }}</td>
            <td>
              <button class="btn-link" type="button" @click="$emit('open-cache', cache)">
                {{ metricValue(cache.size_current) }}
              </button>
            </td>
            <td>
              <button
                class="btn danger tiny"
                :disabled="Boolean(cacheClearingByTag[cache.tag])"
                @click="$emit('clear-cache', cache)"
              >
                {{ cacheClearingByTag[cache.tag] ? '清空中...' : '清空' }}
              </button>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  </section>
</template>
