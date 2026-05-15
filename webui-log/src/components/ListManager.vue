<script setup>
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { getJSON, getText, postJSON } from '../api/http'

const loading = ref(false)
const saving = ref(false)

const selectedTag = ref('')
const content = ref('')
const statusText = ref('未加载')
const specialGroups = ref([])
const listDrafts = ref({})

const fixedProfiles = [
  { tag: 'whitelist', name: '白名单' },
  { tag: 'blocklist', name: '黑名单' },
  { tag: 'greylist', name: '灰名单' },
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

function showTopNotice(message, tone = 'success') {
  if (typeof window === 'undefined') {
    return
  }
  window.dispatchEvent(
    new CustomEvent('mosdns-top-notice', {
      detail: {
        message: String(message || ''),
        tone
      }
    })
  )
}

function setError(message) {
  showTopNotice(message, 'error')
}

function setSuccess(message) {
  showTopNotice(message, 'success')
}

function resetMessage() {
  showTopNotice('', 'success')
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

function getDraft(tag) {
  if (!tag) {
    return null
  }
  return listDrafts.value[tag] || null
}

function ensureDraft(tag, initialContent = '') {
  if (!tag) {
    return null
  }
  if (!listDrafts.value[tag]) {
    listDrafts.value[tag] = {
      original: String(initialContent || ''),
      content: String(initialContent || '')
    }
  }
  return listDrafts.value[tag]
}

function isDraftDirty(tag) {
  const draft = getDraft(tag)
  if (!draft) {
    return false
  }
  return String(draft.content || '') !== String(draft.original || '')
}

function updateStatus(extra = '', tag = selectedTag.value) {
  const draft = getDraft(tag)
  const base = draft ? String(draft.content || '') : String(content.value || '')
  statusText.value = `共 ${lineCount(base)} 行${extra}`
}

async function loadProfiles() {
  resetMessage()
  try {
    const groups = await getJSON('/api/v1/special-groups')
    specialGroups.value = Array.isArray(groups) ? groups : []
  } catch (error) {
    specialGroups.value = []
    setError(`加载专属分流组失败: ${error.message}`)
  }
}

async function loadList(tag, options = {}) {
  if (!tag) {
    return
  }
  const preserveEditing = Boolean(options?.preserveEditing)
  if (preserveEditing && isDraftDirty(tag)) {
    updateStatus('（检测到未保存编辑，已暂停自动刷新）')
    return
  }

  selectedTag.value = tag
  resetMessage()
  const cached = getDraft(tag)
  if (cached && !options?.forceReload) {
    content.value = String(cached.content || '')
    updateStatus(isDraftDirty(tag) ? '（未保存）' : '', tag)
    return
  }

  loading.value = true
  content.value = cached ? String(cached.content || '') : ''
  statusText.value = '加载中...'
  try {
    const text = await getText(`/plugins/${tag}/show?limit=10000`)
    const normalized = String(text || '')
    const draft = ensureDraft(tag, normalized)
    draft.original = normalized
    draft.content = normalized
    if (selectedTag.value === tag) {
      content.value = normalized
      updateStatus('', tag)
    }
  } catch (error) {
    setError(`加载列表失败: ${error.message}`)
    statusText.value = '加载失败'
  } finally {
    loading.value = false
  }
}

async function saveList() {
  if (!selectedTag.value) {
    setError('请先选择列表')
    return
  }

  const pending = Object.entries(listDrafts.value)
    .filter(([_, draft]) => String(draft?.content || '') !== String(draft?.original || ''))
    .map(([tag, draft]) => ({ tag, draft }))

  if (pending.length === 0) {
    setSuccess('没有需要保存的改动')
    return
  }

  saving.value = true
  resetMessage()
  let successCount = 0
  const failed = []
  try {
    for (const item of pending) {
      const values = String(item.draft?.content || '')
        .split('\n')
        .map((value) => value.trim())
        .filter(Boolean)
      try {
        await postJSON(`/plugins/${item.tag}/post`, { values })
        const normalized = values.join('\n')
        item.draft.content = normalized
        item.draft.original = normalized
        successCount += 1
      } catch (error) {
        failed.push({
          tag: item.tag,
          message: String(error?.message || '未知错误')
        })
      }
    }

    const activeDraft = getDraft(selectedTag.value)
    if (activeDraft) {
      content.value = String(activeDraft.content || '')
      updateStatus('', selectedTag.value)
    }

    if (failed.length === 0) {
      setSuccess(`已保存 ${successCount} 个列表改动`)
      return
    }
    const sample = failed.slice(0, 2).map((item) => `${getProfileName(item.tag)}: ${item.message}`).join('；')
    setError(`已保存 ${successCount} 个列表，失败 ${failed.length} 个。${sample}`)
  } finally {
    saving.value = false
  }
}

function onEditorInput() {
  const tag = selectedTag.value
  if (!tag) {
    return
  }
  const draft = ensureDraft(tag, content.value)
  draft.content = String(content.value || '')
  updateStatus(isDraftDirty(tag) ? '（未保存）' : '', tag)
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
    await loadList(selectedTag.value, { preserveEditing: true })
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
  <section class="list-page">
    <div class="list-layout">
      <aside class="list-sidebar">
        <button
          v-for="profile in profiles"
          :key="profile.tag"
          class="list-btn"
          :class="{ active: selectedTag === profile.tag }"
          @click="loadList(profile.tag)"
        >
          {{ profile.name }}<span v-if="isDraftDirty(profile.tag)" class="unsaved-dot"></span>
        </button>
      </aside>

      <main class="list-main">
        <textarea
          v-model="content"
          class="list-editor"
          spellcheck="false"
          :disabled="loading"
          @input="onEditorInput"
          placeholder="每行一个条目"
        />
        <div class="list-footer-row">
          <div class="list-footer-meta">
            <span v-if="selectedHintText" class="list-hint-inline">{{ selectedHintText }}</span>
            <span class="muted list-status-inline">{{ statusText }}</span>
          </div>
          <button class="btn secondary save-list-btn" :disabled="saving || loading" @click="saveList">
            {{ saving ? '保存中...' : '保存全部改动' }}
          </button>
        </div>
      </main>
    </div>
  </section>
</template>
