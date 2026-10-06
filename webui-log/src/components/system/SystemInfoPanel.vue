<script setup>
import { capabilityState, supportsOperation, operationReason } from '../../api/runtimeCapabilities'
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
      <p class="muted" role="note">进程指标 / Go GC / goroutines：{{ operationReason('metrics.process') }}</p>
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
