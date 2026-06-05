<script setup>
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
    <div class="module-kv-list">
      <div class="control-line"><strong>启动时间</strong><span>{{ systemInfo.startTime ? new Date(systemInfo.startTime * 1000).toLocaleString('zh-CN', { hour12: false }) : 'N/A' }}</span></div>
      <div class="control-line"><strong>CPU 时间</strong><span>{{ Number(systemInfo.cpuTime || 0).toFixed(2) }} 秒</span></div>
      <div class="control-line"><strong>常驻内存 (RSS)</strong><span>{{ (Number(systemInfo.residentMemory || 0) / 1024 / 1024).toFixed(2) }} MB</span></div>
      <div class="control-line"><strong>待用堆内存 (Idle)</strong><span>{{ (Number(systemInfo.heapIdleMemory || 0) / 1024 / 1024).toFixed(2) }} MB</span></div>
      <div class="control-line"><strong>Go 版本</strong><span>{{ systemInfo.goVersion }}</span></div>
    </div>
    <div class="actions">
      <button class="btn secondary restart-mosdns-btn" :disabled="restarting" @click="$emit('restart')">
        {{ restarting ? '处理中...' : '重启 MosDNS' }}
      </button>
    </div>
  </section>
</template>
