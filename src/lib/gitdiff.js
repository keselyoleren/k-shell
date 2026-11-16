// Unified diff (`git diff`) → files and lines for the changes panel (components/DiffPanel.vue).

/**
 * Files of a unified diff:
 *   { path, added, removed, isNew, isDeleted, binary,
 *     hunks: [{ skipped, lines: [{ t: ' ' | '+' | '-', text, o, n }] }] }
 * `skipped` is how many unchanged lines come before the hunk (since the last one);
 * `o` / `n` are the line numbers in the old / new file.
 */
export function parseDiff(text) {
  const files = []
  let f = null
  let h = null
  let o = 0
  let n = 0
  let lastOld = 0 // first old line after the previous hunk
  let left = 0
  const unquote = (p) => p.replace(/^"(.*)"$/, '$1')
  for (const line of text.split('\n')) {
    if (line.startsWith('diff --git ')) {
      f = { path: '', added: 0, removed: 0, isNew: false, isDeleted: false, binary: false, hunks: [] }
      const m = /^diff --git "?a\/(.+?)"? "?b\/(.+?)"?$/.exec(line)
      if (m) f.path = m[2]
      files.push(f)
      h = null
      lastOld = 1
      continue
    }
    if (!f) continue
    if (!h) {
      if (line.startsWith('new file mode')) f.isNew = true
      else if (line.startsWith('deleted file mode')) f.isDeleted = true
      else if (line.startsWith('Binary files')) f.binary = true
      else if (line.startsWith('+++ ') && line !== '+++ /dev/null') f.path = unquote(line.slice(4)).replace(/^b\//, '')
      else if (line.startsWith('--- ') && line !== '--- /dev/null' && !f.path)
        f.path = unquote(line.slice(4)).replace(/^a\//, '')
    }
    const hh = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/.exec(line)
    if (hh) {
      o = +hh[1]
      n = +hh[3]
      const start = +hh[2] === 0 ? o + 1 : o // "-0,0": nothing before
      h = { skipped: Math.max(0, start - lastOld), lines: [] }
      left = (hh[2] == null ? 1 : +hh[2]) + (hh[4] == null ? 1 : +hh[4]) // lines the hunk still has, old + new
      f.hunks.push(h)
      continue
    }
    if (!h) continue
    // an empty unchanged line can come without its leading space (diff.suppressBlankEmpty)
    const t = line === '' && left > 1 ? ' ' : line[0]
    if (t === '+') left--
    else if (t === '-') left--
    else if (t === ' ') left -= 2
    if (t === '+') {
      h.lines.push({ t, text: line.slice(1), o: null, n: n++ })
      f.added++
    } else if (t === '-') {
      h.lines.push({ t, text: line.slice(1), o: o++, n: null })
      f.removed++
    } else if (t === ' ') {
      h.lines.push({ t, text: line.slice(1), o: o++, n: n++ })
    } else continue // "\ No newline at end of file", or the end
    lastOld = o
  }
  return files.filter((x) => x.path)
}

/** "src/components/" and "CommandBar.vue" */
export function splitPath(p) {
  const i = p.lastIndexOf('/')
  return [p.slice(0, i + 1), p.slice(i + 1)]
}
