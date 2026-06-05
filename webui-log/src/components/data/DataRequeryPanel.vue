<script setup>
defineProps({
  isRequeryRunning: {
    type: Boolean,
    default: false
  },
  lastRunErrorText: {
    type: String,
    default: ''
  },
  lastRunText: {
    type: String,
    default: ''
  },
  requeryAction: {
    type: String,
    default: ''
  },
  requeryAvailable: {
    type: Boolean,
    default: true
  },
  requeryLoadError: {
    type: String,
    default: ''
  },
  requeryProgress: {
    type: Object,
    required: true
  },
  requeryStatusMeta: {
    type: Object,
    required: true
  },
  schedulerForm: {
    type: Object,
    required: true
  },
  schedulerSaving: {
    type: Boolean,
    default: false
  }
})

defineEmits([
  'trigger',
  'cancel',
  'update-scheduler',
  'schedule-scheduler-update',
  'save-rules',
  'clear-rules'
])
</script>

<template>
  <section class="panel sub-panel data-module data-inline-module requery-module">
    <header class="panel-header">
      <div>
        <h3>刷新分流缓存</h3>
        <p class="muted">任务状态、进度与定时配置。</p>
      </div>
    </header>

    <p v-if="requeryLoadError" class="muted">{{ requeryLoadError }}</p>

    <template v-if="requeryAvailable">
      <div class="requery-status-head">
        <div class="requery-status-item">
          <strong>当前状态</strong>
          <span class="requery-status-chip" :class="requeryStatusMeta.className">{{ requeryStatusMeta.text }}</span>
        </div>
        <div class="requery-status-item">
          <strong>上次运行</strong>
          <span>{{ lastRunText }}</span>
        </div>
        <div class="actions">
          <button
            v-if="!isRequeryRunning"
            class="btn primary"
            :disabled="requeryAction === 'trigger'"
            @click="$emit('trigger')"
          >
            {{ requeryAction === 'trigger' ? '启动中...' : '开始全新任务' }}
          </button>
          <button
            v-else
            class="btn danger"
            :disabled="requeryAction === 'cancel'"
            @click="$emit('cancel')"
          >
            {{ requeryAction === 'cancel' ? '取消中...' : '取消任务' }}
          </button>
        </div>
      </div>
      <p v-if="lastRunErrorText" class="muted">最近失败原因：{{ lastRunErrorText }}</p>

      <div v-if="isRequeryRunning" class="requery-progress-wrap">
        <div class="requery-progress-bar">
          <div class="requery-progress-bar-fill" :style="{ width: `${requeryProgress.percent}%` }"></div>
          <span class="requery-progress-text">
            {{ Math.floor(requeryProgress.percent) }}% ({{ requeryProgress.processed.toLocaleString() }} / {{ requeryProgress.total.toLocaleString() }})
          </span>
        </div>
      </div>

      <div class="requery-scheduler">
        <div class="scheduler-row">
          <span>启用定时任务</span>
          <label class="switch">
            <input v-model="schedulerForm.enabled" type="checkbox" @change="$emit('update-scheduler')" />
            <span class="slider"></span>
          </label>
        </div>
        <div class="scheduler-grid">
          <label>首次执行时间</label>
          <input
            v-model="schedulerForm.startDatetimeLocal"
            class="scheduler-field"
            type="datetime-local"
            :disabled="!schedulerForm.enabled"
            @change="$emit('schedule-scheduler-update')"
          />

          <label>间隔 (分钟)</label>
          <input
            v-model.number="schedulerForm.intervalMinutes"
            class="scheduler-field"
            type="number"
            min="1"
            :disabled="!schedulerForm.enabled"
            @change="$emit('schedule-scheduler-update')"
          />

          <label>域名刷新天数</label>
          <input
            v-model.number="schedulerForm.dateRangeDays"
            class="scheduler-field"
            type="number"
            min="1"
            @change="$emit('schedule-scheduler-update')"
          />
        </div>
        <p class="muted">{{ schedulerSaving ? '定时配置保存中...' : '修改后自动保存配置。' }}</p>
      </div>

      <div class="requery-important">
        <strong>重要操作</strong>
        <div class="actions">
          <button class="btn primary" :disabled="requeryAction === 'save-rules'" @click="$emit('save-rules')">
            {{ requeryAction === 'save-rules' ? '保存中...' : '保存分流规则' }}
          </button>
          <button class="btn danger" :disabled="requeryAction === 'clear-rules'" @click="$emit('clear-rules')">
            {{ requeryAction === 'clear-rules' ? '清空中...' : '清空分流规则' }}
          </button>
        </div>
      </div>
    </template>
  </section>
</template>
