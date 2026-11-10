// Helpers for the server stats panel (components/HostStats.vue). Raw numbers come from the
// `host_stats` command; these turn them into percentages, rates and short labels.

/**
 * CPU usage from two /proc/stat samples: { busy, user, system, iowait, steal } in %,
 * or null when they can't be compared (first sample, counter reset).
 */
export function cpuUsage(prev, cur) {
  if (!prev || !cur) return null
  const total = cur.total - prev.total
  if (total <= 0 || cur.idle < prev.idle) return null
  const p = (k) => Math.min(100, Math.max(0, (100 * (cur[k] - prev[k])) / total))
  return {
    busy: Math.min(100, Math.max(0, (100 * (total - (cur.idle - prev.idle))) / total)),
    user: p('user'),
    system: p('system'),
    iowait: p('iowait'),
    steal: p('steal'),
  }
}

/** Bytes per second between two counter samples taken `ms` apart, or null. */
export function rate(prev, cur, ms) {
  if (prev == null || cur == null || ms <= 0 || cur < prev) return null
  return ((cur - prev) * 1000) / ms
}

/** 0–100, or null. */
export function pct(used, total) {
  return total ? Math.min(100, Math.max(0, (100 * used) / total)) : null
}

/** "3d 4h", "5h 12m", "8m" */
export function fmtUptime(secs) {
  if (secs == null) return ''
  const d = Math.floor(secs / 86400),
    h = Math.floor((secs % 86400) / 3600),
    m = Math.floor((secs % 3600) / 60)
  if (d) return `${d}d ${h}h`
  if (h) return `${h}h ${m}m`
  return `${m}m`
}

/** Bytes in binary units with one decimal for small values: "7.6 GB", "512 MB". */
export function fmtBytes(n) {
  if (n == null) return ''
  const u = ['B', 'KB', 'MB', 'GB', 'TB', 'PB']
  let i = 0
  while (n >= 1024 && i < u.length - 1) {
    n /= 1024
    i++
  }
  return `${n >= 10 || i === 0 ? Math.round(n) : n.toFixed(1)} ${u[i]}`
}

/** "1.2 MB/s" */
export const fmtRate = (n) => (n == null ? '–' : fmtBytes(n) + '/s')

/** "23%" */
export const fmtPct = (p) => (p == null ? '–' : Math.round(p) + '%')

/** Colour level for a percentage. */
export const level = (p) => (p == null ? '' : p >= 90 ? 'crit' : p >= 75 ? 'warn' : '')

/** Keep the last `max` values of a series. */
export function push(series, v, max = 60) {
  series.push(v)
  if (series.length > max) series.splice(0, series.length - max)
  return series
}
