<script setup>
import { computed } from 'vue'
import { badge } from '../lib/os'

const props = defineProps({ host: Object })
const b = computed(() => badge(props.host))
</script>

<template>
  <span class="badge" :style="{ background: b.bg, color: b.fg }" :title="b.logo?.title">
    <svg v-if="b.logo" viewBox="0 0 24 24" aria-hidden="true"><path :d="b.logo.path" fill="currentColor" /></svg>
    <template v-else>{{ b.text }}</template>
  </span>
</template>

<style scoped>
svg {
  width: 58%;
  height: 58%;
}
</style>
