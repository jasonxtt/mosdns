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

const supportsRuleSyntax = '支持 full:, domain:, keyword:, regexp: 等规则格式。'

const profiles = computed(() => {
  const dynamic = [...specialGroups.value]
    .sort((a, b) => Number(a.slot) - Number(b.slot))
    .map((g) => ({
      tag: g.manual_plugin_tag || `special_manual_${g.slot}`,
      name: g.name || `专属分流组 ${g.slot}`
    }))
  return [...fixedProfiles, ...dynamic]
})

const selectedHintText = computed(() => {
  const tag = selectedTag.value
  if (!tag) {
    return ''
  }

  switch (tag) {
    case 'whitelist':
      return `此列表中的域名会优先命中白名单规则，通过国内DNS解析。${supportsRuleSyntax}`
    case 'blocklist':
      return `此列表中的域名会优先命中黑名单规则并被屏蔽。${supportsRuleSyntax}`
    case 'greylist':
      return `此列表中的域名会优先命中灰名单规则，通过国外DNS（fakeip）解析。${supportsRuleSyntax}`
    case 'ddnslist':
      return `此列表中的域名会按 DDNS 域名处理，适合动态域名解析场景。${supportsRuleSyntax}`
    case 'client_ip':
      return '打开此开关：系统-功能开关-指定 Client fakeip/指定 Client realip，同时mosdns作为dns下发给客户端，此名单/功能才生效；生效时，只有指定的客户端可以获取fakeip/指定客户端不可以获取fakeip。'
    case 'direct_ip':
      return '不在任何域名清单中的域名解析后的IP属于此IP清单时，此域名向被归入直连域名。以苹果公司IP段为例：17.0.0.0/8'
    case 'rewrite':
      return '格式: <域名> <IP或域名>。例如: example.com 1.2.3.4 或 test.com example.com。支持 full:, domain: 等匹配规则。'
    case 'realiplist':
      return '在此名单中的域名向国外DNS解析并返回真实 IP (RealIP)，不使用 FakeIP。适用于必须使用真实 IP 连接的域名。'
    case 'cnfakeipfilter':
      return '在此名单中的域名向国内DNS解析并返回真实 IP (RealIP)，不使用 FakeIP。适用于必须使用真实 IP 连接的域名。'
    default: {
      const profile = profiles.value.find((item) => item.tag === tag)
      if (!profile) {
        return ''
      }
      const isFixed = fixedProfiles.some((item) => item.tag === tag)
      if (isFixed) {
        return ''
      }
      return `此列表中的域名会直接归入“${profile.name}”专属分流组，并使用该组绑定的专属上游与缓存。${supportsRuleSyntax}`
    }
  }
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
  return trimmed.split('\n').map((line) => line.trim()).filter(Boolean).length
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
    statusText.value = `共 ${lineCount(content.value)} 行`
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
    statusText.value = `共 ${values.length} 行`
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
        <p v-if="selectedHintText" class="list-hint">{{ selectedHintText }}</p>
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
