<script setup>
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { getJSON, getText, postJSON } from '../api/http'

const loading = ref(false)
const saving = ref(false)
const errorMessage = ref('')
const successMessage = ref('')

const selectedTag = ref('')
const content = ref('')
const statusText = ref('未加载')
const specialGroups = ref([])

const fixedProfiles = [
  { tag: 'whitelist', name: '白名单' },
  { tag: 'blocklist', name: '黑名单' },
  { tag: 'greylist', name: '灰名单' },
  { tag: 'realiplist', name: '!CN fakeip filter' },
  { tag: 'cnfakeipfilter', name: 'CN fakeip filter' },
  { tag: 'ddnslist', name: 'DDNS 域名' },
  { tag: 'client_ip', name: '客户端 IP' },
  { tag: 'direct_ip', name: '直连 IP' },
  { tag: 'rewrite', name: '重定向' }
]

const profiles = computed(() => {
  const dynamic = [...specialGroups.value]
    .sort((a, b) => Number(a.slot) - Number(b.slot))
    .map((g) => ({
      tag: g.manual_plugin_tag || `special_manual_${g.slot}`,
      name: g.name || `专属分流组 ${g.slot}`
    }))
  return [...fixedProfiles, ...dynamic]
})

function resetMessage() {
  errorMessage.value = ''
  successMessage.value = ''
}

function getProfileName(tag) {
  return profiles.value.find((p) => p.tag === tag)?.name || tag
}

function lineCount(text) {
  const trimmed = text.trim()
  if (!trimmed) {
    return 0
  }
  return trimmed.split('\n').length
}

async function loadProfiles() {
  resetMessage()
  try {
    const groups = await getJSON('/api/v1/special-groups')
    specialGroups.value = Array.isArray(groups) ? groups : []
  } catch (error) {
    specialGroups.value = []
    errorMessage.value = `加载专属分流组失败: ${error.message}`
  }
}

async function loadList(tag) {
  if (!tag) {
    return
  }
  selectedTag.value = tag
  loading.value = true
  resetMessage()
  content.value = ''
  statusText.value = '加载中...'
  try {
    const text = await getText(`/plugins/${tag}/show?limit=10000`)
    content.value = text || ''
    statusText.value = `已加载 ${lineCount(content.value)} 行`
  } catch (error) {
    errorMessage.value = `加载列表失败: ${error.message}`
    statusText.value = '加载失败'
  } finally {
    loading.value = false
  }
}

async function saveList() {
  if (!selectedTag.value) {
    errorMessage.value = '请先选择列表'
    return
  }
  saving.value = true
  resetMessage()
  try {
    const values = content.value
      .split('\n')
      .map((item) => item.trim())
      .filter(Boolean)
    await postJSON(`/plugins/${selectedTag.value}/post`, { values })
    successMessage.value = `列表“${getProfileName(selectedTag.value)}”已保存`
    statusText.value = `已保存 ${values.length} 行`
  } catch (error) {
    errorMessage.value = `保存失败: ${error.message}`
  } finally {
    saving.value = false
  }
}

async function init() {
  await loadProfiles()
  if (!selectedTag.value && profiles.value.length > 0) {
    await loadList(profiles.value[0].tag)
  }
}

async function handleGlobalRefresh() {
  await loadProfiles()
  if (selectedTag.value) {
    await loadList(selectedTag.value)
  }
}

onMounted(() => {
  init()
  window.addEventListener('mosdns-log-refresh', handleGlobalRefresh)
})

onBeforeUnmount(() => {
  window.removeEventListener('mosdns-log-refresh', handleGlobalRefresh)
})
</script>

<template>
  <section class="panel">
    <header class="panel-header">
      <div>
        <h2>名单管理</h2>
        <p class="muted">管理固定名单和专属分流组绑定名单。当前仓库功能范围与现有后端插件保持一致。</p>
      </div>
      <div class="actions">
        <button class="btn primary" :disabled="saving || loading" @click="saveList">
          {{ saving ? '保存中...' : '保存列表' }}
        </button>
      </div>
    </header>

    <p v-if="errorMessage" class="msg error">{{ errorMessage }}</p>
    <p v-if="successMessage" class="msg success">{{ successMessage }}</p>

    <div class="list-layout">
      <aside class="list-sidebar">
        <button
          v-for="profile in profiles"
          :key="profile.tag"
          class="list-btn"
          :class="{ active: selectedTag === profile.tag }"
          @click="loadList(profile.tag)"
        >
          {{ profile.name }}
        </button>
      </aside>

      <main class="list-main">
        <header class="list-main-header">
          <strong>{{ selectedTag ? getProfileName(selectedTag) : '未选择' }}</strong>
          <span class="muted">{{ statusText }}</span>
        </header>
        <textarea
          v-model="content"
          class="list-editor"
          spellcheck="false"
          :disabled="loading"
          placeholder="每行一个条目"
        />
      </main>
    </div>
  </section>
</template>
