<script setup>
// A small history graph: one or more series drawn right-aligned, newest value at the right edge.
import { computed } from 'vue'

const props = defineProps({
  /** [{ values: number[], color: css colour }] */
  series: Array,
  /** Top of the scale; null = the largest value shown. */
  max: { type: Number, default: null },
  /** Number of samples across the full width. */
  points: { type: Number, default: 60 },
  height: { type: Number, default: 34 },
})

const W = 100
const top = computed(() => {
  if (props.max != null) return props.max
  let m = 0
  for (const s of props.series) for (const v of s.values) if (v > m) m = v
  return m || 1
})
function coords(values) {
  const step = W / Math.max(1, props.points - 1)
  const start = W - (values.length - 1) * step
  return values.map((v, i) => [
    start + i * step,
    props.height - (Math.min(v ?? 0, top.value) / top.value) * (props.height - 2) - 1,
  ])
}
const paths = computed(() =>
  props.series.map((s) => {
    const c = coords(s.values)
    if (c.length < 2) return { ...s, line: '', area: '' }
    const line = c.map(([x, y], i) => `${i ? 'L' : 'M'}${x.toFixed(2)} ${y.toFixed(2)}`).join('')
    return {
      ...s,
      line,
      area: `${line}L${c.at(-1)[0].toFixed(2)} ${props.height}L${c[0][0].toFixed(2)} ${props.height}Z`,
    }
  }),
)
</script>

<template>
  <svg class="spark" :viewBox="`0 0 ${W} ${height}`" preserveAspectRatio="none" :style="{ height: height + 'px' }">
    <g v-for="(p, i) in paths" :key="i">
      <!-- colours as styles: presentation attributes don't resolve var(--…) -->
      <path :d="p.area" :style="{ fill: p.color, fillOpacity: 0.16 }" />
      <path
        :d="p.line"
        :style="{ stroke: p.color }"
        fill="none"
        stroke-width="1.4"
        vector-effect="non-scaling-stroke"
      />
    </g>
  </svg>
</template>

<style scoped>
.spark {
  display: block;
  width: 100%;
  border-radius: 4px;
  background: rgba(127, 127, 127, 0.08);
}
</style>
