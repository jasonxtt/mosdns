<script setup>
import { ref } from 'vue'

const panelBackgroundPicker = ref(null)

defineProps({
  appearance: {
    type: Object,
    required: true
  },
  buttonColorDraft: {
    type: String,
    default: '#000000'
  },
  buttonColorSaving: {
    type: Boolean,
    default: false
  },
  formatRelativeTime: {
    type: Function,
    required: true
  },
  panelBackground: {
    type: Object,
    required: true
  },
  panelBackgroundHistory: {
    type: Array,
    default: () => []
  },
  panelBackgroundHistoryBusy: {
    type: String,
    default: ''
  },
  panelBackgroundHistoryLoading: {
    type: Boolean,
    default: false
  },
  panelBackgroundHistoryOpen: {
    type: Boolean,
    default: false
  },
  textColorDraft: {
    type: String,
    default: '#000000'
  },
  textColorSaving: {
    type: Boolean,
    default: false
  },
  themeOptions: {
    type: Array,
    default: () => []
  }
})

defineEmits([
  'apply-theme',
  'text-color-input',
  'text-color-change',
  'pick-text-color',
  'reset-text-color',
  'button-color-input',
  'button-color-change',
  'pick-button-color',
  'reset-button-color',
  'panel-bg-color-input',
  'panel-bg-url-enter',
  'open-panel-bg-picker',
  'panel-bg-file-change',
  'toggle-panel-bg-history',
  'clear-panel-bg-history',
  'use-panel-bg-history',
  'delete-panel-bg-history',
  'panel-bg-slider-input',
  'apply-panel-bg-settings',
  'reset-appearance-settings'
])

function openPanelBackgroundPicker() {
  panelBackgroundPicker.value?.click()
}
</script>

<template>
  <div class="appearance-settings control-module-wide">
    <section class="panel settings-card appearance-settings-card">
      <h3>主题与外观</h3>
      <div class="form-grid theme-grid">
        <div class="form-item">
          <label>界面风格</label>
          <select v-model="appearance.theme" @change="$emit('apply-theme', appearance.theme)">
            <option v-for="opt in themeOptions" :key="opt.value" :value="opt.value">{{ opt.label }}</option>
          </select>
        </div>

        <div class="form-item">
          <label>字体颜色</label>
          <div class="appearance-color-control">
            <div class="appearance-color-shell">
              <input
                :value="textColorDraft"
                type="color"
                :disabled="textColorSaving"
                @input="$emit('text-color-input', $event)"
                @change="$emit('text-color-change', $event)"
              />
              <button class="btn tiny secondary appearance-color-inline-btn" type="button" :disabled="textColorSaving" @click="$emit('reset-text-color')">默认</button>
            </div>
          </div>
        </div>

        <div class="form-item">
          <label>按钮颜色</label>
          <div class="appearance-color-control">
            <div class="appearance-color-shell">
              <input
                :value="buttonColorDraft"
                type="color"
                :disabled="buttonColorSaving"
                @input="$emit('button-color-input', $event)"
                @change="$emit('button-color-change', $event)"
              />
              <button class="btn tiny secondary appearance-color-inline-btn" type="button" :disabled="buttonColorSaving" @click="$emit('reset-button-color')">默认</button>
            </div>
          </div>
        </div>
      </div>
    </section>

    <section class="panel settings-card appearance-settings-card">
      <h3>面板背景</h3>

      <div class="form-item">
        <label>背景图</label>
        <div class="background-input-row">
          <div class="background-url-field">
            <div class="appearance-background-input">
              <input
                class="panel-bg-color-inline appearance-background-swatch"
                :value="appearance.theme === 'dark' ? panelBackground.darkColor : panelBackground.lightColor"
                type="color"
                :disabled="panelBackground.uploading || panelBackground.applying"
                title="纯色"
                @input="$emit('panel-bg-color-input', $event)"
              />
              <input
                v-model="panelBackground.url"
                class="appearance-background-url"
                placeholder="输入图片URL链接"
                @keydown.enter.prevent="$emit('panel-bg-url-enter')"
              />
            </div>
          </div>
          <button class="btn tiny secondary" type="button" :disabled="panelBackground.uploading || panelBackground.applying" @click="openPanelBackgroundPicker">
            {{ panelBackground.uploading ? '上传中' : '上传' }}
          </button>
          <button class="btn tiny secondary" type="button" :disabled="panelBackground.uploading || panelBackground.applying || panelBackgroundHistoryLoading" @click="$emit('toggle-panel-bg-history')">
            {{ panelBackgroundHistoryOpen ? '收起' : '记录' }}
          </button>
        </div>
        <input
          ref="panelBackgroundPicker"
          class="panel-bg-file-input"
          type="file"
          accept="image/*"
          @change="$emit('panel-bg-file-change', $event)"
        />
      </div>

      <div v-if="panelBackgroundHistoryOpen" class="panel-bg-history appearance-settings-history">
        <div class="panel-bg-history-head">
          <strong>历史图片</strong>
          <button class="btn tiny danger" type="button" :disabled="panelBackgroundHistoryBusy === 'clear-all'" @click="$emit('clear-panel-bg-history')">
            {{ panelBackgroundHistoryBusy === 'clear-all' ? '清空中...' : '清空历史' }}
          </button>
        </div>
        <p v-if="panelBackgroundHistoryLoading" class="muted">历史加载中...</p>
        <p v-else-if="panelBackgroundHistory.length === 0" class="muted">暂无历史图片</p>
        <div v-else class="panel-bg-history-list">
          <div v-for="item in panelBackgroundHistory" :key="item.id" class="panel-bg-history-item">
            <img class="panel-bg-history-thumb" :src="item.image_url" alt="history background" />
            <div class="panel-bg-history-meta">
              <div class="mono">{{ item.id }}</div>
              <div class="muted">{{ formatRelativeTime(item.modified_at) }}</div>
            </div>
            <div class="panel-bg-history-actions">
              <button class="btn tiny secondary" type="button" :disabled="panelBackgroundHistoryBusy === item.id" @click="$emit('use-panel-bg-history', item)">选用</button>
              <button class="btn tiny danger" type="button" :disabled="panelBackgroundHistoryBusy === item.id" @click="$emit('delete-panel-bg-history', item)">
                {{ panelBackgroundHistoryBusy === item.id ? '删除中...' : '删除' }}
              </button>
            </div>
          </div>
        </div>
      </div>

      <div class="range-grid">
        <div class="form-item">
          <label>透明度</label>
          <div class="panel-bg-range-wrap appearance-range-row range-row">
            <input v-model.number="panelBackground.transparency" type="range" min="0" max="100" step="1" @input="$emit('panel-bg-slider-input')" />
            <span class="range-value">{{ Number(panelBackground.transparency || 0) }}%</span>
          </div>
        </div>

        <div class="form-item">
          <label>毛玻璃</label>
          <div class="panel-bg-range-wrap appearance-range-row range-row">
            <input v-model.number="panelBackground.blur" type="range" min="0" max="40" step="1" @input="$emit('panel-bg-slider-input')" />
            <span class="range-value">{{ Number(panelBackground.blur || 0) }}px</span>
          </div>
        </div>
      </div>

      <div class="card-actions">
        <button class="btn tiny secondary" type="button" :disabled="panelBackground.applying || panelBackground.uploading || textColorSaving || buttonColorSaving" @click="$emit('reset-appearance-settings')">重置</button>
        <button class="btn tiny primary" type="button" :disabled="panelBackground.applying || panelBackground.uploading" @click="$emit('apply-panel-bg-settings')">
          {{ panelBackground.applying ? '应用中...' : '应用' }}
        </button>
      </div>
    </section>
  </div>
</template>
