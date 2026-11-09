import { LOGOS } from './logos'

// Visual identity for hosts: OS / protocol badge colour + short label.
const OS = {
  ubuntu: ['#e95420', 'U'],
  debian: ['#a80030', 'D'],
  centos: ['#932279', 'C'],
  rhel: ['#cc0000', 'R'],
  fedora: ['#294172', 'F'],
  rocky: ['#10b981', 'R'],
  almalinux: ['#0f4266', 'A'],
  arch: ['#1793d1', 'A'],
  alpine: ['#0d597f', 'A'],
  amzn: ['#ff9900', 'A'],
  opensuse: ['#73ba25', 'S'],
  linux: ['#475569', 'L'],
  darwin: ['#6b7280', ''],
  freebsd: ['#ab2b28', 'B'],
  raspbian: ['#c51a4a', 'P'],
  kali: ['#367bf0', 'K'],
}
// Muted, earthy colours; a host always gets the same one (derived from its name).
const PALETTE = ['#C8553D', '#2F5D8A', '#5B7F4E', '#8A5A44', '#6B5B95', '#B07D2B', '#3E7C7B', '#7A4E6B']
function pick(name) {
  let h = 0
  for (const c of name) h = (h * 31 + c.charCodeAt(0)) >>> 0
  return PALETTE[h % PALETTE.length]
}

// /etc/os-release ID (or `uname -s`, lowercased) → logo in ./logos.js
const OS_LOGO = {
  ubuntu: 'ubuntu',
  debian: 'debian',
  centos: 'centos',
  rhel: 'redhat',
  fedora: 'fedora',
  rocky: 'rockylinux',
  almalinux: 'almalinux',
  arch: 'archlinux',
  alpine: 'alpinelinux',
  opensuse: 'opensuse',
  'opensuse-leap': 'opensuse',
  'opensuse-tumbleweed': 'opensuse',
  sles: 'suse',
  sled: 'suse',
  linux: 'linux',
  darwin: 'apple',
  freebsd: 'freebsd',
  openbsd: 'openbsd',
  raspbian: 'raspberrypi',
  kali: 'kalilinux',
  linuxmint: 'linuxmint',
  manjaro: 'manjaro',
  gentoo: 'gentoo',
  nixos: 'nixos',
}
// S3-compatible endpoint host → provider logo. AWS (no endpoint) keeps the S3 badge.
const S3_LOGO = [
  [/\.r2\.cloudflarestorage\.com$/, 'cloudflare'],
  [/digitaloceanspaces\.com$/, 'digitalocean'],
  [/wasabisys\.com$/, 'wasabi'],
  [/backblazeb2\.com$/, 'backblaze'],
  [/^storage\.googleapis\.com$/, 'googlecloud'],
  [/\.scw\.cloud$/, 'scaleway'],
  [/your-objectstorage\.com$/, 'hetzner'],
  [/linodeobjects\.com$/, 'akamai'],
  [/minio/, 'minio'],
]
// Black brand colours vanish on the dark theme: these keep the badge colour they had.
const LOGO_BG = { apple: '#6b7280', almalinux: '#0f4266' }

function endpointHost(endpoint) {
  try {
    return new URL(/^[a-z]+:\/\//i.test(endpoint) ? endpoint : 'https://' + endpoint).hostname.toLowerCase()
  } catch {
    return ''
  }
}

/** The logo (key of LOGOS) for a host: its provider for object storage, its OS for SSH. */
export function logoFor(h) {
  if (h.protocol === 'gcs') return 'googlecloud'
  if (h.protocol === 's3') {
    const host = endpointHost(h.endpoint || '')
    return host ? S3_LOGO.find(([re]) => re.test(host))?.[1] || null : null
  }
  if (!h.protocol || h.protocol === 'ssh') return OS_LOGO[h.os] || null
  return null
}

/** Dark text on light colours (Linux yellow…), white otherwise. */
function ink(bg) {
  const m = /^#([0-9a-f]{6})$/i.exec(bg || '')
  if (!m) return '#fff'
  const n = parseInt(m[1], 16)
  const [r, g, b] = [n >> 16, (n >> 8) & 255, n & 255].map((c) => {
    c /= 255
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4
  })
  return 0.2126 * r + 0.7152 * g + 0.0722 * b > 0.45 ? '#1d1d1f' : '#fff'
}

/** Badge for a host: background, text colour, and a brand logo or a short label. */
export function badge(h) {
  const b = baseBadge(h)
  const key = logoFor(h)
  const logo = key && LOGOS[key]
  if (logo) {
    b.logo = logo
    const custom = h.color && (!h.protocol || h.protocol === 'ssh') // a colour picked for an SSH host wins
    if (!custom) b.bg = LOGO_BG[key] || logo.hex
  }
  b.fg = ink(b.bg)
  return b
}

function baseBadge(h) {
  if (h.protocol === 's3') return { bg: '#C27C2C', text: 'S3', icon: 'cloud' }
  if (h.protocol === 'azblob') return { bg: '#2F6DB5', text: 'AZ', icon: 'cloud' }
  if (h.protocol === 'gcs') return { bg: '#3E7C7B', text: 'GCS', icon: 'cloud' }
  if (h.protocol === 'ftp' || h.protocol === 'ftps') return { bg: '#6B5B95', text: 'FTP', icon: 'folder' }
  if (h.color) return { bg: h.color, text: (h.label || h.host || '?')[0].toUpperCase() }
  const o = OS[h.os]
  if (o) return { bg: o[0], text: o[1] || (h.label || '?')[0].toUpperCase() }
  return {
    bg: pick(h.label || h.host || ''),
    text: (h.label || h.host || '?')[0].toUpperCase(),
  }
}
export const PROTOCOLS = [
  { id: 'ssh', label: 'SSH / SFTP', port: 22 },
  { id: 'ftp', label: 'FTP', port: 21 },
  { id: 'ftps', label: 'FTPS (explicit TLS)', port: 21 },
  { id: 's3', label: 'S3-compatible', port: 0 },
  { id: 'azblob', label: 'Azure Blob', port: 0 },
  { id: 'gcs', label: 'Google Cloud Storage', port: 0 },
]

/** Protocols that browse buckets/containers instead of a server's file system. */
export const OBJECT_PROTOCOLS = ['s3', 'azblob', 'gcs']
export const isObjectStorage = (protocol) => OBJECT_PROTOCOLS.includes(protocol)
// `url`: 'path' = https://endpoint/bucket, 'virtual' = https://bucket.endpoint.
// Values in <angle brackets> differ per account or region and must be filled in.
export const S3_PRESETS = [
  { label: 'AWS S3', endpoint: '', region: 'us-east-1', url: 'virtual' },
  { label: 'Cloudflare R2', endpoint: 'https://<account-id>.r2.cloudflarestorage.com', region: 'auto', url: 'path' },
  { label: 'MinIO', endpoint: 'http://localhost:9000', region: 'us-east-1', url: 'path' },
  { label: 'DigitalOcean Spaces', endpoint: 'https://sgp1.digitaloceanspaces.com', region: 'sgp1', url: 'path' },
  { label: 'Wasabi', endpoint: 'https://s3.ap-southeast-1.wasabisys.com', region: 'ap-southeast-1', url: 'path' },
  { label: 'Backblaze B2', endpoint: 'https://s3.us-west-004.backblazeb2.com', region: 'us-west-004', url: 'path' },
  { label: 'Google Cloud (HMAC keys)', endpoint: 'https://storage.googleapis.com', region: 'auto', url: 'path' },
  { label: 'Tencent COS', endpoint: 'https://cos.<region>.myqcloud.com', region: '<region>', url: 'virtual' },
  { label: 'Scaleway', endpoint: 'https://s3.<region>.scw.cloud', region: '<region>', url: 'path' },
  { label: 'Hetzner', endpoint: 'https://<location>.your-objectstorage.com', region: '<location>', url: 'path' },
  { label: 'Linode / Akamai', endpoint: 'https://<region>.linodeobjects.com', region: '<region>', url: 'path' },
  {
    label: 'Oracle Cloud',
    endpoint: 'https://<namespace>.compat.objectstorage.<region>.oraclecloud.com',
    region: '<region>',
    url: 'path',
  },
]
