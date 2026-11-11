// Terminal recordings (asciicast, `.cast`): a recorder fed by a terminal pane, and a parser
// for the player. Files are written by the backend (src-tauri/src/cast.rs).

import { api, uid } from './api'

/**
 * Records what a terminal pane shows: output (`o`) and resizes (`r`), never typed input.
 * Events are sent to the backend in small batches, so long recordings don't pile up in memory.
 */
export class Recorder {
  constructor() {
    this.id = uid()
    this.started = 0
    this.queue = []
    this.decoder = new TextDecoder('utf-8') // stream: true keeps characters split across chunks
    this.chain = Promise.resolve()
    this.timer = 0
    this.failed = null
    this.path = ''
  }
  async start({ label, cols, rows }) {
    this.path = await api.rec_start({ id: this.id, label, cols, rows })
    this.started = performance.now()
    this.timer = setInterval(() => this.flush(), 500)
    return this.path
  }
  /** Seconds since the start. */
  elapsed() {
    return this.started ? (performance.now() - this.started) / 1000 : 0
  }
  out(bytes) {
    const text = typeof bytes === 'string' ? bytes : this.decoder.decode(bytes, { stream: true })
    if (text) this.queue.push([this.elapsed(), 'o', text])
  }
  resize(cols, rows) {
    this.queue.push([this.elapsed(), 'r', `${cols}x${rows}`])
  }
  flush() {
    if (!this.queue.length) return this.chain
    const events = this.queue.splice(0)
    this.chain = this.chain.then(() =>
      api.rec_write({ id: this.id, events }).catch((e) => {
        this.failed ||= String(e)
      }),
    )
    return this.chain
  }
  /** Close the file and return its path. */
  async stop() {
    clearInterval(this.timer)
    const tail = this.decoder.decode()
    if (tail) this.queue.push([this.elapsed(), 'o', tail])
    await this.chain
    const events = this.queue.splice(0)
    return api.rec_stop({ id: this.id, events })
  }
}

/**
 * Parse an asciicast file (v1, v2 or v3) into
 * `{ cols, rows, title, idleLimit, duration, events: [[seconds, 'o' | 'r', data]] }`
 * with absolute times. Input, marker and exit events are dropped.
 */
export function parseCast(text) {
  text = text.replace(/^﻿/, '')
  const lines = text.split(/\r?\n/).filter((l) => l.trim())
  if (!lines.length) throw new Error('This file is empty')
  let header
  try {
    header = JSON.parse(lines[0])
  } catch {
    throw new Error('Not an asciinema recording (.cast)')
  }
  const events = []
  if (header.version === 1) {
    let t = 0
    for (const [d, data] of header.stdout || []) events.push([(t += +d || 0), 'o', String(data)])
    return finish(header.width, header.height, header, events)
  }
  if (header.version !== 2 && header.version !== 3) throw new Error('Not an asciinema recording (.cast)')
  const v3 = header.version === 3
  let t = 0
  for (const l of lines.slice(1)) {
    if (l.startsWith('#')) continue // v3 comments
    let e
    try {
      e = JSON.parse(l)
    } catch {
      continue // a recording cut off mid-line still plays up to there
    }
    if (!Array.isArray(e) || e.length < 3) continue
    t = v3 ? t + (+e[0] || 0) : +e[0] || 0
    if (e[1] === 'o' || e[1] === 'r') events.push([t, e[1], String(e[2])])
  }
  const cols = v3 ? header.term?.cols : header.width
  const rows = v3 ? header.term?.rows : header.height
  return finish(cols, rows, header, events)
}

function finish(cols, rows, header, events) {
  events.sort((a, b) => a[0] - b[0])
  return {
    cols: +cols || 80,
    rows: +rows || 24,
    title: header.title || '',
    idleLimit: +header.idle_time_limit || null,
    duration: events.length ? events.at(-1)[0] : 0,
    events,
  }
}

/** Shorten every pause longer than `limit` seconds to `limit` (like asciinema's idle_time_limit). */
export function limitIdle(events, limit) {
  if (!limit) return events
  let shift = 0,
    last = 0
  return events.map(([t, k, d]) => {
    const gap = t - last
    if (gap > limit) shift += gap - limit
    last = t
    return [t - shift, k, d]
  })
}

/** "1:05", "1:02:09" */
export function fmtClock(secs) {
  secs = Math.max(0, Math.floor(secs || 0))
  const h = Math.floor(secs / 3600),
    m = Math.floor((secs % 3600) / 60),
    s = String(secs % 60).padStart(2, '0')
  return h ? `${h}:${String(m).padStart(2, '0')}:${s}` : `${m}:${s}`
}
