<script setup>
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { getJSON, getText, postJSON } from '../api/http'
import { getRuntimeCapabilities, supportsOperation, operationReason } from '../api/runtimeCapabilities'
import { loadNativeLocalProfiles } from '../api/nativeManagement'
import { clearTopNotice, setError, setSuccess } from '../utils/notice'

const loading = ref(false)
const saving = ref(false)

const selectedTag = ref('')
const content = ref('')
const statusText = ref('未加载')
const specialGroups = ref([])
const runtimeCapabilities = ref(null)
const listDrafts = ref({})
// Tags whose POST returned 200 but whose canonical `/show` reread failed. The
// server mutation may have happened, but the UI does not know which rules were
// accepted, so those tags keep their local draft and stay out of the
// confirmed-saved count until a reread succeeds.
const uncertainTags = ref([])

const isNative = computed(() => runtimeCapabilities.value?.kind === 'native')
const canSaveList = computed(() => supportsOperation('rules.local.manage'))

const nativeFixedProfiles = ref([])

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
  if (!runtimeCapabilities.value) return []
  return isNative.value ? (runtimeCapabilities.value?.special_groups?.enabled ? dynamic : nativeFixedProfiles.value) : [...fixedProfiles, ...dynamic]
})

const selectedHintText = computed(() => {
  const tag = selectedTag.value
  if (!tag) {
    return ''
  }

  if (isNative.value) {
    if (!runtimeCapabilities.value?.special_groups?.enabled) return '原生本地文件规则；按每行一个规则编辑，保存后由后端验证。'
    const profile = profiles.value.find((item) => item.tag === tag)
    return profile
      ? `此列表绑定到“${profile.name}”专属分流组。按每行一个规则编辑；规则由原生运行时验证后保存。`
      : ''
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
  clearTopNotice()
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

function isUncertain(tag) {
  return uncertainTags.value.includes(tag)
}

function markUncertain(tag) {
  if (!uncertainTags.value.includes(tag)) {
    uncertainTags.value = [...uncertainTags.value, tag]
  }
}

function clearUncertain(tag) {
  if (uncertainTags.value.includes(tag)) {
    uncertainTags.value = uncertainTags.value.filter((item) => item !== tag)
  }
}

function showUrl(tag) {
  return `/plugins/${tag}/show?limit=10000`
}

// The server returns one accepted rule per line with a trailing newline, while
// the editor content is whatever the user typed. Comparing the effective rule
// lists keeps a trailing newline or a blank line from looking like a change.
function canonicalRules(text) {
  return String(text || '')
    .split('\n')
    .map((value) => value.trim())
    .filter(Boolean)
    .join('\n')
}

function sameRules(left, right) {
  return canonicalRules(left) === canonicalRules(right)
}

// Reads the canonical server state for one tag. Only a successful read may
// clear a tag's dirty state, because the server skips rules it rejects.
async function fetchCanonical(tag) {
  return String((await getText(showUrl(tag))) || '')
}

function updateStatus(extra = '', tag = selectedTag.value) {
  const draft = getDraft(tag)
  const base = draft ? String(draft.content || '') : String(content.value || '')
  statusText.value = `共 ${lineCount(base)} 行${extra}`
}

async function loadProfiles() {
  resetMessage()
  try {
    if (isNative.value && !runtimeCapabilities.value?.special_groups?.enabled) {
      specialGroups.value = []
      if (supportsOperation('rules.local.read')) nativeFixedProfiles.value = await loadNativeLocalProfiles(fixedProfiles, getText)
      return
    }
    if (!supportsOperation('groups.read')) return
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
  // An uncertain tag always retries the canonical read, so a later load
  // reconciles the server state instead of trusting a cached draft.
  if (cached && !options?.forceReload && !isUncertain(tag)) {
    content.value = String(cached.content || '')
    updateStatus(isDraftDirty(tag) ? '（未保存）' : '', tag)
    return
  }

  loading.value = true
  content.value = cached ? String(cached.content || '') : ''
  statusText.value = '加载中...'
  try {
    const normalized = await fetchCanonical(tag)
    const draft = ensureDraft(tag, normalized)
    if (isUncertain(tag)) {
      // Reconciliation: adopt the server text as the baseline but keep a local
      // edit when it differs, so the user decides whether to submit it again.
      const localContent = String(draft.content || '')
      const hadLocalEdits = !sameRules(localContent, draft.original)
      clearUncertain(tag)
      draft.original = normalized
      if (!hadLocalEdits || sameRules(localContent, normalized)) {
        draft.content = normalized
      } else {
        setError(
          `「${getProfileName(tag)}」服务器内容与本地编辑不同，已保留本地编辑，请确认后再保存`
        )
      }
    } else {
      draft.original = normalized
      draft.content = normalized
    }
    if (selectedTag.value === tag) {
      content.value = String(draft.content || '')
      updateStatus(isDraftDirty(tag) ? '（未保存）' : '', tag)
    }
  } catch (error) {
    if (isUncertain(tag)) {
      setError(`「${getProfileName(tag)}」当前内容仍未确认: ${error.message}`)
    } else {
      setError(`加载列表失败: ${error.message}`)
    }
    statusText.value = '加载失败'
  } finally {
    loading.value = false
  }
}

async function saveList() {
  if (!canSaveList.value) {
    setError('当前原生运行时不支持保存手工规则')
    return
  }
  if (!selectedTag.value) {
    setError('请先选择列表')
    return
  }

  const unclean = Object.entries(listDrafts.value)
    .filter(
      ([tag, draft]) =>
        String(draft?.content || '') !== String(draft?.original || '') || isUncertain(tag)
    )
    .map(([tag, draft]) => ({ tag, draft }))

  if (unclean.length === 0) {
    setSuccess('没有需要保存的改动')
    return
  }

  saving.value = true
  resetMessage()
  let confirmedCount = 0
  const adjusted = []
  const preserved = []
  const unconfirmed = []
  const failed = []
  try {
    for (const item of unclean) {
      const tag = item.tag

      // An uncertain tag retries the canonical read first. A POST is never
      // repeated blindly, and a local edit that differs from the server is
      // kept for the user to confirm.
      if (isUncertain(tag)) {
        let canonical
        try {
          canonical = await fetchCanonical(tag)
        } catch (error) {
          unconfirmed.push({ tag, message: String(error?.message || '未知错误') })
          continue
        }
        clearUncertain(tag)
        item.draft.original = canonical
        if (!sameRules(item.draft.content, canonical)) {
          // The editor keeps the local edit; this is not a server-side
          // adjustment, so it is reported as a separate state.
          preserved.push({ tag, message: '服务器内容与本地编辑不同，已保留本地编辑' })
          continue
        }
        item.draft.content = canonical
        confirmedCount += 1
        continue
      }

      const values = String(item.draft?.content || '')
        .split('\n')
        .map((value) => value.trim())
        .filter(Boolean)
      try {
        await postJSON(`/plugins/${tag}/post`, { values })
      } catch (error) {
        failed.push({
          tag,
          message: String(error?.message || '未知错误')
        })
        continue
      }

      // POST 200 means the server mutated. Only a successful canonical reread
      // confirms which rules the server actually accepted.
      let canonical
      try {
        canonical = await fetchCanonical(tag)
      } catch (error) {
        markUncertain(tag)
        unconfirmed.push({ tag, message: String(error?.message || '未知错误') })
        continue
      }

      const submitted = values.join('\n')
      item.draft.original = canonical
      item.draft.content = canonical
      if (!sameRules(canonical, submitted)) {
        adjusted.push({ tag, message: '部分规则未被服务器接受，已按服务器内容更新' })
      }
      confirmedCount += 1
    }

    const activeDraft = getDraft(selectedTag.value)
    if (activeDraft) {
      content.value = String(activeDraft.content || '')
      updateStatus(isDraftDirty(selectedTag.value) ? '（未保存）' : '', selectedTag.value)
    }

    if (
      failed.length === 0 &&
      unconfirmed.length === 0 &&
      adjusted.length === 0 &&
      preserved.length === 0
    ) {
      setSuccess(`已保存 ${confirmedCount} 个列表改动`)
      return
    }
    const names = (items) =>
      items
        .slice(0, 2)
        .map((item) => getProfileName(item.tag))
        .join('、')
    const parts = [`已保存 ${confirmedCount} 个列表`]
    if (adjusted.length > 0) {
      parts.push(`${adjusted.length} 个列表部分规则未被服务器接受，已按服务器内容更新（${names(adjusted)}）`)
    }
    if (preserved.length > 0) {
      parts.push(
        `${preserved.length} 个列表已重新读取服务器内容，但本地编辑与服务器不同，已保留本地编辑（${names(preserved)}），请确认后再保存`
      )
    }
    if (unconfirmed.length > 0) {
      parts.push(
        `${unconfirmed.length} 个列表服务器保存成功但当前内容未确认（${names(unconfirmed)}），请重新加载确认`
      )
    }
    if (failed.length > 0) {
      const sample = failed
        .slice(0, 2)
        .map((item) => `${getProfileName(item.tag)}: ${item.message}`)
        .join('；')
      parts.push(`${failed.length} 个列表保存失败（${sample}）`)
    }
    setError(parts.join('；'))
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
  try {
    runtimeCapabilities.value = await getRuntimeCapabilities()
  } catch (error) {
    setError(`读取运行时能力失败: ${error.message}`)
    return
  }
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
    <p v-if="isNative && profiles.length === 0" class="muted" role="note">
      当前原生运行时未启用专属组本地规则管理。
    </p>
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
          <span
            v-if="isUncertain(profile.tag)"
            class="unsaved-dot"
            style="background: #e6a23c"
            title="服务器保存成功，当前内容未确认"
          ></span>
        </button>
      </aside>

      <main class="list-main">
        <p v-if="!canSaveList" class="muted" role="note" data-operation-reason="rules.local.manage">{{ operationReason('rules.local.manage') }}</p>
        <textarea
          v-model="content"
          class="list-editor"
          spellcheck="false"
          :disabled="loading"
          :readonly="!canSaveList"
          @input="onEditorInput"
          placeholder="每行一个条目"
        />
        <div class="list-footer-row">
          <div class="list-footer-meta">
            <span v-if="selectedHintText" class="list-hint-inline">{{ selectedHintText }}</span>
            <span class="muted list-status-inline">{{ statusText }}</span>
          </div>
          <button class="btn secondary save-list-btn" :disabled="saving || loading || !canSaveList" @click="saveList">
            {{ saving ? '保存中...' : '保存全部改动' }}
          </button>
        </div>
      </main>
    </div>
  </section>
</template>
