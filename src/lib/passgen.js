// Random passwords from the OS CSPRNG (crypto.getRandomValues), with no modulo bias.

const SETS = {
  lower: 'abcdefghijklmnopqrstuvwxyz',
  upper: 'ABCDEFGHIJKLMNOPQRSTUVWXYZ',
  digits: '0123456789',
  symbols: '!@#$%^&*()-_=+[]{};:,.?/~',
}
/** Characters that are easy to misread: 0 O o, 1 l I |. */
const AMBIGUOUS = /[0Oo1lI|]/g

export const DEFAULT_OPTS = { length: 24, lower: true, upper: true, digits: true, symbols: true, avoidAmbiguous: false }

/** Uniform random integer in [0, n). */
function randInt(n) {
  const limit = Math.floor(0x100000000 / n) * n
  const buf = new Uint32Array(1)
  do crypto.getRandomValues(buf)
  while (buf[0] >= limit)
  return buf[0] % n
}

function pools(o) {
  return Object.keys(SETS)
    .filter((k) => o[k])
    .map((k) => (o.avoidAmbiguous ? SETS[k].replace(AMBIGUOUS, '') : SETS[k]))
}

/** A password with at least one character from every chosen set. */
export function generatePassword(opts = {}) {
  const o = { ...DEFAULT_OPTS, ...opts }
  const sets = pools(o)
  if (!sets.length) return ''
  const length = Math.max(o.length, sets.length)
  const all = sets.join('')
  const chars = sets.map((s) => s[randInt(s.length)])
  while (chars.length < length) chars.push(all[randInt(all.length)])
  for (let i = chars.length - 1; i > 0; i--) {
    const j = randInt(i + 1)
    ;[chars[i], chars[j]] = [chars[j], chars[i]]
  }
  return chars.join('')
}

/** Rough strength of a password: entropy in bits from its length and the kinds of characters in it. */
export function strength(pw) {
  if (!pw) return { bits: 0, label: '', level: 0 }
  let size = 0
  if (/[a-z]/.test(pw)) size += 26
  if (/[A-Z]/.test(pw)) size += 26
  if (/[0-9]/.test(pw)) size += 10
  if (/[^a-zA-Z0-9]/.test(pw)) size += 33
  const bits = Math.round(pw.length * Math.log2(size || 1))
  const level = bits < 40 ? 1 : bits < 60 ? 2 : bits < 80 ? 3 : 4
  return { bits, level, label: ['', 'Weak', 'Fair', 'Good', 'Strong'][level] }
}
