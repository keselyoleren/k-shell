<script setup>
import { computed } from 'vue'
import { fileIcon, pagePath } from '../lib/fileicons'

const props = defineProps({ name: String, size: { type: [Number, String], default: 15 } })
const icon = computed(() => fileIcon(props.name || ''))
</script>

<template>
  <svg
    :width="size"
    :height="size"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="1.8"
    stroke-linecap="round"
    stroke-linejoin="round"
    class="icon fileicon"
    :style="icon.color ? { color: icon.color } : null"
    aria-hidden="true"
  >
    <path :d="pagePath(icon.kind)" />
  </svg>
</template>
