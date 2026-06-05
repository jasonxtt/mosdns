<script setup>
defineProps({
  lastRunDomainCountText: {
    type: String,
    default: '--'
  },
  listStats: {
    type: Array,
    default: () => []
  }
})

defineEmits(['open-list'])
</script>

<template>
  <section class="panel sub-panel data-module data-inline-module domain-stats-module">
    <header class="panel-header">
      <div>
        <h3>域名列表统计</h3>
        <p class="muted">展示动态分流列表当前条目数。</p>
      </div>
    </header>

    <div class="table-wrap domain-stats-table-wrap data-scroll-wrap">
      <table class="domain-stats-table">
        <thead>
          <tr>
            <th>列表名称</th>
            <th>条目数</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="row in listStats" :key="row.key">
            <td>{{ row.name }}</td>
            <td>
              <span v-if="row.error" class="mono">{{ row.error }}</span>
              <span v-else-if="row.count === null">--</span>
              <button v-else class="btn-link" type="button" @click="$emit('open-list', row)">
                {{ Number(row.count).toLocaleString() }}
              </button>
            </td>
          </tr>
          <tr>
            <td><strong>刷新域名</strong></td>
            <td><strong class="mono">{{ lastRunDomainCountText }}</strong></td>
          </tr>
        </tbody>
      </table>
    </div>
  </section>
</template>
