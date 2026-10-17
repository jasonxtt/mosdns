<script setup>
import { computed } from 'vue'
import { capabilityState, supportsOperation, operationReason } from '../../api/runtimeCapabilities'

// The native host lists every directory under `<config root>/ui/` in its health (served at /<name>/).
const externalEntries = computed(() => (Array.isArray(capabilityState.health?.external_ui) ? capabilityState.health.external_ui : []))
// The directory this page was loaded from ('/' for the built-in UI, '/<name>/' for an external one).
const currentDirectory = typeof window !== 'undefined' ? new URL('./', window.location.href).pathname : '/'
const packageUrl = (entry) => `/${encodeURIComponent(entry.name)}/`
// Compatible mosdns UI packages other than the one being viewed.
const mountedPackages = computed(() => externalEntries.value.filter((entry) => entry.kind === 'mosdns_ui' && entry.mounted && packageUrl(entry) !== currentDirectory))
// Packages the host refused (incompatible, broken manifest, ...), with the reason.
const refusedPackages = computed(() => externalEntries.value.filter((entry) => entry.kind === 'mosdns_ui' && !entry.mounted))

defineProps({
  restarting: {
    type: Boolean,
    default: false
  },
  systemInfo: {
    type: Object,
    required: true
  }
})

defineEmits(['restart'])
</script>

<template>
  <section class="panel control-module control-module--mini">
    <h3>系统信息</h3>
    <div v-if="capabilityState.value?.kind === 'native'" class="module-kv-list">
      <div class="control-line"><strong>运行时</strong><span>Rust</span></div>
      <div class="control-line"><strong>产品版本</strong><span>{{ supportsOperation('system.version') ? (capabilityState.health?.version || '读取中') : operationReason('system.version') }}</span></div>
      <p v-for="entry in mountedPackages" :key="`ui-${entry.name}`" class="muted" role="note" data-external-ui-link>检测到外置 UI「{{ entry.name }}」版本 {{ entry.ui_version }}，<a :href="packageUrl(entry)">前往 {{ packageUrl(entry) }}</a></p>
      <p v-for="entry in refusedPackages" :key="`refused-${entry.name}`" class="muted" role="note" data-external-ui-rejected>外置 UI「{{ entry.name }}」未挂载：{{ entry.reason }}</p>
      <template v-if="supportsOperation('metrics.process')">
        <div class="control-line"><strong>启动时间</strong><span>{{ systemInfo.startTime ? new Date(systemInfo.startTime * 1000).toLocaleString('zh-CN', { hour12: false }) : 'N/A' }}</span></div>
        <div class="control-line"><strong>CPU 时间</strong><span>{{ Number(systemInfo.cpuTime || 0).toFixed(2) }} 秒</span></div>
        <div class="control-line"><strong>常驻内存 (RSS)</strong><span>{{ (Number(systemInfo.residentMemory || 0) / 1024 / 1024).toFixed(2) }} MB</span></div>
        <div class="control-line"><strong>打开文件描述符</strong><span>{{ Number(systemInfo.openFds || 0).toLocaleString() }}</span></div>
        <div class="control-line"><strong>线程数</strong><span>{{ Number(systemInfo.threads || 0).toLocaleString() }}</span></div>
        <p class="muted" role="note">GC、Go 版本、goroutines：不适用于 Rust 运行时</p>
      </template>
      <p v-else class="muted" role="note">进程指标 / Go GC / goroutines：{{ operationReason('metrics.process') }}</p>
    </div>
    <div v-else class="module-kv-list">
      <div class="control-line"><strong>启动时间</strong><span>{{ systemInfo.startTime ? new Date(systemInfo.startTime * 1000).toLocaleString('zh-CN', { hour12: false }) : 'N/A' }}</span></div>
      <div class="control-line"><strong>CPU 时间</strong><span>{{ Number(systemInfo.cpuTime || 0).toFixed(2) }} 秒</span></div>
      <div class="control-line"><strong>常驻内存 (RSS)</strong><span>{{ (Number(systemInfo.residentMemory || 0) / 1024 / 1024).toFixed(2) }} MB</span></div>
      <div class="control-line"><strong>待用堆内存 (Idle)</strong><span>{{ (Number(systemInfo.heapIdleMemory || 0) / 1024 / 1024).toFixed(2) }} MB</span></div>
      <div class="control-line"><strong>Go 版本</strong><span>{{ systemInfo.goVersion }}</span></div>
    </div>
    <p v-if="!supportsOperation('system.restart')" class="muted" role="note">{{ operationReason('system.restart') }}</p>
    <div class="actions">
      <button class="btn secondary restart-mosdns-btn" :disabled="restarting || !supportsOperation('system.restart')" @click="$emit('restart')">
        {{ restarting ? '处理中...' : '重启 MosDNS' }}
      </button>
    </div>
  </section>
</template>
