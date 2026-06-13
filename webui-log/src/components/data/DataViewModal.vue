<script setup>
defineProps({
  dataView: {
    type: Object,
    required: true
  }
})

defineEmits(['close', 'search-input', 'load-more'])
</script>

<template>
  <Teleport to="body">
    <div class="modal-mask" @click.self="$emit('close')">
      <section class="panel data-view-modal">
        <header class="panel-header">
          <div>
            <h3>{{ dataView.title }}</h3>
            <p class="muted">
              <span v-if="dataView.totalCount > 0">当前显示 {{ dataView.entries.length.toLocaleString() }} / {{ dataView.totalCount.toLocaleString() }} 条</span>
              <span v-else>当前显示 {{ dataView.entries.length.toLocaleString() }} 条</span>
            </p>
          </div>
          <div class="actions">
            <button class="btn secondary" type="button" @click="$emit('close')">关闭</button>
          </div>
        </header>

      <div class="data-view-search">
        <input
          v-model="dataView.query"
          placeholder="搜索..."
          @input="$emit('search-input')"
        />
      </div>

      <p v-if="dataView.error" class="muted">{{ dataView.error }}</p>

      <div v-if="dataView.mode === 'domain'" class="table-wrap">
        <table>
          <thead>
            <tr>
              <th>次数</th>
              <th>最后日期</th>
              <th>域名</th>
            </tr>
          </thead>
          <tbody>
            <tr v-if="dataView.loading">
              <td colspan="3" class="empty">加载中...</td>
            </tr>
            <tr v-else-if="dataView.entries.length === 0">
              <td colspan="3" class="empty">没有匹配的条目</td>
            </tr>
            <tr v-for="(item, index) in dataView.entries" :key="`domain-entry-${index}`">
              <td>{{ item.count }}</td>
              <td>{{ item.date }}</td>
              <td>{{ item.domain }}</td>
            </tr>
          </tbody>
        </table>
      </div>

      <div v-else class="cache-entry-list">
        <p v-if="dataView.loading" class="empty">加载中...</p>
        <p v-else-if="dataView.entries.length === 0" class="empty">没有匹配的条目</p>
        <details
          v-for="item in dataView.entries"
          :key="item.key"
          class="cache-entry"
        >
          <summary>{{ item.headerTitle }}</summary>
          <div class="table-wrap cache-meta-wrap">
            <table>
              <tbody>
                <tr v-for="(meta, index) in item.metadataRows" :key="`${item.key}-meta-${index}`">
                  <td>{{ meta.key }}</td>
                  <td class="mono">{{ meta.value }}</td>
                </tr>
              </tbody>
            </table>
          </div>
          <pre class="mono cache-pre">{{ item.dnsMessage }}</pre>
        </details>
      </div>

        <div class="actions" style="margin-top: 10px;">
          <button
            class="btn primary"
            type="button"
            :disabled="dataView.loading || dataView.loadingMore || !dataView.hasMore"
            @click="$emit('load-more')"
          >
            {{ dataView.loadingMore ? '加载中...' : '加载更多' }}
          </button>
        </div>
      </section>
    </div>
  </Teleport>
</template>
