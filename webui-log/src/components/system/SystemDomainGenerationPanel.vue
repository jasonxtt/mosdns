<script setup>
defineProps({
  domainGenerationProfiles: {
    type: Array,
    default: () => [],
  },
  domainGenerationLoading: {
    type: Boolean,
    default: false,
  },
  domainGenerationSettings: {
    type: Object,
    required: true,
  },
});

defineEmits(["toggle-domain-generation"]);
</script>

<template>
  <section class="panel control-module domain-generation-module">
    <header class="module-head">
      <h3>域名表自生成</h3>
    </header>

    <div class="switch-list domain-generation-switch-list">
      <label
        v-for="profile in domainGenerationProfiles"
        :key="profile.key"
        class="switch-row"
      >
        <div class="switch-meta">
          <strong>{{ profile.name }}</strong>
          <span class="muted">{{ profile.desc }}</span>
        </div>
        <span class="switch switch-compact">
          <input
            type="checkbox"
            :checked="Boolean(domainGenerationSettings[profile.key])"
            :disabled="domainGenerationLoading"
            @change="
              $emit('toggle-domain-generation', profile, $event.target.checked)
            "
          />
          <span class="slider"></span>
        </span>
      </label>
    </div>
  </section>
</template>
