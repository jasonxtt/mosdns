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
  cacheRows: {
    type: Array,
    default: () => []
  }
})

defineEmits(['clear-all', 'open-cache', 'clear-cache'])
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
            <td colspan="8" class="empty">暂无缓存数据</td>
          </tr>
          <tr v-for="cache in cacheRows" :key="cache.key">
            <td>{{ cache.name }}</td>
            <td>{{ Number(cache.query_total || 0).toLocaleString() }}</td>
            <td>{{ Number(cache.hit_total || 0).toLocaleString() }}</td>
            <td>{{ Number(cache.lazy_hit_total || 0).toLocaleString() }}</td>
            <td>{{ cache.hit_rate }}</td>
            <td>{{ cache.lazy_hit_rate }}</td>
            <td>
              <button class="btn-link" type="button" @click="$emit('open-cache', cache)">
                {{ Number(cache.size_current || 0).toLocaleString() }}
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
