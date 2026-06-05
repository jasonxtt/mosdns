<script setup>
defineProps({
  isSwitchChecked: {
    type: Function,
    required: true
  },
  secondarySwitches: {
    type: Array,
    default: () => []
  },
  switchLoading: {
    type: Object,
    required: true
  },
  switchStates: {
    type: Object,
    required: true
  }
})

defineEmits(['toggle-switch'])
</script>

<template>
  <section class="panel control-module control-module-wide">
    <header class="module-head">
      <div>
        <h3>功能开关</h3>
      </div>
    </header>

    <div class="switch-list">
      <label v-for="profile in secondarySwitches" :key="profile.tag" class="switch-row">
        <div class="switch-meta">
          <strong>{{ profile.name }}</strong>
          <span class="muted">{{ profile.desc }}</span>
        </div>
        <span class="switch switch-compact">
          <input
            type="checkbox"
            :checked="isSwitchChecked(profile)"
            :disabled="switchLoading[profile.tag] || switchStates[profile.tag] === 'error'"
            @change="$emit('toggle-switch', profile, $event.target.checked)"
          />
          <span class="slider"></span>
        </span>
      </label>
    </div>
  </section>
</template>
