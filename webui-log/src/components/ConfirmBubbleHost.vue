<script setup>
import { onBeforeUnmount, onMounted } from 'vue'
import { closeConfirm, confirmState } from '../utils/confirm'

function handleCancel() {
  closeConfirm(false)
}

function handleConfirm() {
  closeConfirm(true)
}

function handleEsc(event) {
  if (event.key === 'Escape' && confirmState.open) {
    event.preventDefault()
    closeConfirm(false)
  }
}

onMounted(() => {
  window.addEventListener('keydown', handleEsc)
})

onBeforeUnmount(() => {
  window.removeEventListener('keydown', handleEsc)
})
</script>

<template>
  <div v-if="confirmState.open" class="confirm-pop-mask" @click.self="handleCancel">
    <section class="confirm-pop-bubble" role="dialog" aria-modal="true" aria-label="确认框">
      <h4>{{ confirmState.title }}</h4>
      <p>{{ confirmState.message }}</p>
      <div class="confirm-pop-actions">
        <button class="btn secondary" type="button" @click="handleCancel">{{ confirmState.cancelText }}</button>
        <button class="btn" :class="confirmState.tone === 'danger' ? 'danger' : 'primary'" type="button" @click="handleConfirm">
          {{ confirmState.confirmText }}
        </button>
      </div>
    </section>
  </div>
</template>
