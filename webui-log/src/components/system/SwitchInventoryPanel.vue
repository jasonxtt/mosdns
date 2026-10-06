<script setup>
import { reactive, watch } from 'vue'

const props = defineProps({
  instances: { type: Array, default: () => [] },
  states: { type: Object, required: true },
  loading: { type: Object, required: true },
})

const emit = defineEmits(['save'])
const drafts = reactive({})

function syncDrafts(instances = props.instances) {
  for (const instance of instances) {
    if (!Object.hasOwn(drafts, instance.tag)) drafts[instance.tag] = String(props.states[instance.tag] ?? '')
  }
}

watch(() => props.instances, syncDrafts, { immediate: true, deep: true })
watch(() => props.states, syncDrafts, { deep: true })

function save(instance) {
  emit('save', { tag: instance.tag, value: String(drafts[instance.tag] ?? '') })
}
</script>

<template>
  <section class="panel control-module native-switch-inventory-panel">
    <header class="module-head">
      <div>
        <h3>已配置开关</h3>
        <p class="muted">状态由当前配置规则使用；原生模式不推断产品功能或缓存副作用。</p>
      </div>
    </header>
    <div class="native-switch-inventory-list">
      <div v-for="instance in instances" :key="instance.tag" class="native-switch-row">
        <div class="native-switch-meta">
          <strong>{{ instance.type }} · {{ instance.tag }}</strong>
          <span class="muted">{{ instance.writable ? '可读写' : (instance.reason || '只读') }}</span>
        </div>
        <div class="native-switch-editor">
          <input
            v-model="drafts[instance.tag]"
            class="input native-switch-value"
            :disabled="!instance.readable || !instance.writable || loading[instance.tag]"
            :aria-label="`${instance.tag} 状态值`"
          />
          <button
            type="button"
            class="btn tiny primary"
            :disabled="!instance.readable || !instance.writable || loading[instance.tag]"
            @click="save(instance)"
          >
            {{ loading[instance.tag] ? '保存中…' : '保存' }}
          </button>
        </div>
        <code class="native-switch-readback">当前：{{ states[instance.tag] ?? '读取中…' }}</code>
      </div>
      <p v-if="!instances.length" class="muted">当前配置没有可管理的开关。</p>
    </div>
  </section>
</template>

<style scoped>
.native-switch-inventory-panel { padding: 14px; }
.native-switch-inventory-list { display: grid; gap: 10px; }
.native-switch-row { display: grid; grid-template-columns: minmax(150px, .8fr) minmax(220px, 1fr) minmax(120px, .7fr); gap: 10px; align-items: center; padding: 10px 0; border-top: 1px solid var(--line); }
.native-switch-row:first-child { border-top: 0; }
.native-switch-meta { display: grid; gap: 3px; min-width: 0; }
.native-switch-meta strong { overflow-wrap: anywhere; }
.native-switch-editor { display: flex; gap: 8px; min-width: 0; }
.native-switch-value { min-width: 0; flex: 1 1 auto; }
.native-switch-readback { min-width: 0; overflow-wrap: anywhere; color: var(--ink-1); }
@media (max-width: 760px) {
  .native-switch-row { grid-template-columns: 1fr; }
}
</style>
