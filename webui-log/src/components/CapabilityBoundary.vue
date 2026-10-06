<script setup>
import { supportsOperation, operationReason } from '../api/runtimeCapabilities'
defineProps({ operation: { type: String, required: true } })
</script>
<template>
  <fieldset class="capability-boundary" :disabled="!supportsOperation(operation)" :data-operation="operation">
    <p v-if="!supportsOperation(operation)" class="muted capability-reason" role="note">{{ operationReason(operation) }}</p>
    <slot />
  </fieldset>
</template>
<style scoped>
.capability-boundary { border: 0; padding: 0; margin: 0; min-width: 0; }
.capability-reason { margin: 0 0 8px; }
</style>
