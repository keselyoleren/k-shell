// File manager icons: a page outline with a mark for the kind of file, coloured by type.
// Drawn in the same 24×24, 1.8-stroke style as Icon.vue.

import { extOf } from './filetypes'
import { LANG_LOGOS } from './langlogos'

const PAGE = 'M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9zM14 3v6h6'

/** The mark drawn inside the page, per kind. */
export const MARKS = {
  file: '',
  text: 'M8 13h8M8 17h5',
  code: 'M10 12.5 8 15l2 2.5M14 12.5l2 2.5-2 2.5',
  script: 'M8 13l2.5 2L8 17M12.5 17.5H16',
  config: 'M8 13h8M8 17h8M10.5 11.5v3M13.5 15.5v3',
  data: 'M8 12.5h1.5M8 15h1.5M8 17.5h1.5M12 12.5h4M12 15h4M12 17.5h4',
  image: 'M7.5 18l3-3.5 2 2 1.5-1.5 2.5 3M9.5 12.5h.01',
  video: 'M10 12.5v5l4.5-2.5z',
  audio: 'M11 17.5v-5.5l4-1v5M11 17.5a1.5 1.5 0 1 1-3 0 1.5 1.5 0 0 1 3 0zM15 16a1.5 1.5 0 1 1-3 0 1.5 1.5 0 0 1 3 0z',
  archive: 'M10 5v1.5M10 8.5V10M10 12v1.5M8.5 15.5h3V19h-3z',
  pdf: 'M8 13h8M8 17h8M8 15h5',
  sheet: 'M8 12h8v6.5H8zM8 15.2h8M12 12v6.5',
  slides: 'M7.5 12.5h9v5h-9zM12 17.5v1.5',
  key: 'M11.5 16a2 2 0 1 1-4 0 2 2 0 0 1 4 0zM11.5 16H16.5M15 16v2',
  lock: 'M9 15h6v3.5H9zM10.2 15v-1.3a1.8 1.8 0 0 1 3.6 0V15',
  database: 'M8 13c0 .9 1.8 1.5 4 1.5s4-.6 4-1.5-1.8-1.5-4-1.5-4 .6-4 1.5zM8 13v4.5c0 .9 1.8 1.5 4 1.5s4-.6 4-1.5V13',
  font: 'M9 18.5l3-7 3 7M10.2 16h3.6',
  binary: 'M12 11.5l3.5 2v4L12 19.5l-3.5-2v-4zM8.5 13.5 12 15.5l3.5-2M12 15.5v4',
  git: 'M9 12v6M9 15.5c3 0 5-1 5-3.5M14 12a.01.01 0 0 1 0 0M9 18a.01.01 0 0 1 0 0',
  docker: 'M7.5 15.5h9c-.3 2-2 3-4.5 3s-4-1-4.5-3zM9 15.5v-2h2v2M11 15.5v-2h2v2M11 13.5v-2h2v2',
}
export const pagePath = (kind) => PAGE + (MARKS[kind] ? MARKS[kind] : '')

// Colours: languages use their own colour (toned so it reads on dark and light themes).
const C = {
  text: '#8b95a7',
  md: '#519aba',
  pdf: '#e5484d',
  doc: '#2b79d8',
  sheet: '#21a366',
  slides: '#d24726',
  image: '#a074c4',
  video: '#e45c86',
  audio: '#e8a33d',
  archive: '#b8893a',
  key: '#d4a72c',
  lock: '#8b95a7',
  db: '#8e7cc3',
  font: '#b0739a',
  binary: '#7d8796',
  config: '#6d8a9c',
  shell: '#4eaa25',
  js: '#d4b106',
  ts: '#3178c6',
  py: '#3776ab',
  rs: '#ce7b4c',
  go: '#00add8',
  java: '#e76f00',
  kotlin: '#a97bff',
  c: '#659ad2',
  cs: '#a179dc',
  php: '#777bb4',
  rb: '#cc342d',
  swift: '#f05138',
  vue: '#41b883',
  svelte: '#ff3e00',
  html: '#e34f26',
  css: '#1572b6',
  sass: '#cc6699',
  json: '#cba200',
  yaml: '#cb4b4b',
  xml: '#e37933',
  sql: '#e38c00',
  lua: '#5a6bc4',
  dart: '#0175c2',
  docker: '#2496ed',
  git: '#f05032',
  tf: '#7b42bc',
  r: '#276dc3',
  ex: '#8f6bb5',
  hs: '#8f7fcc',
  zig: '#f7a41d',
  nix: '#7e7eff',
}

// extension → [kind, colour]
const BY_EXT = {}
const add = (exts, kind, color) => exts.split(' ').forEach((e) => (BY_EXT[e] = [kind, color]))
add('txt log out', 'text', C.text)
add('md markdown mdx rst adoc', 'text', C.md)
add('pdf', 'pdf', C.pdf)
add('doc docx odt rtf pages epub', 'text', C.doc)
add('xls xlsx ods numbers', 'sheet', C.sheet)
add('csv tsv', 'data', C.sheet)
add('ppt pptx odp key', 'slides', C.slides)
add('png jpg jpeg gif webp bmp ico svg avif heic tif tiff psd raw', 'image', C.image)
add('mp4 mkv mov avi webm wmv m4v flv', 'video', C.video)
add('mp3 wav flac ogg m4a aac opus wma', 'audio', C.audio)
add('zip tar gz tgz bz2 xz zst 7z rar jar war deb rpm apk dmg iso pkg msi whl lz4', 'archive', C.archive)
add('pem crt cer der p12 pfx ppk pub csr gpg asc', 'key', C.key)
add('lock', 'lock', C.lock)
add('sqlite sqlite3 db mdb parquet', 'database', C.db)
add('ttf otf woff woff2 eot', 'font', C.font)
add('exe dll so dylib bin o a class pyc wasm app', 'binary', C.binary)
add('ini conf cfg config properties env editorconfig', 'config', C.config)
add('toml', 'config', C.config)
add('sh bash zsh fish ksh command', 'script', C.shell)
add('ps1 psm1 bat cmd', 'script', C.cs)
add('js mjs cjs jsx', 'code', C.js)
add('ts mts cts tsx', 'code', C.ts)
add('py pyw pyi ipynb', 'code', C.py)
add('rs', 'code', C.rs)
add('go', 'code', C.go)
add('java', 'code', C.java)
add('kt kts', 'code', C.kotlin)
add('c h cpp cc cxx hpp hh m mm', 'code', C.c)
add('cs fs vb', 'code', C.cs)
add('php', 'code', C.php)
add('rb erb gemspec', 'code', C.rb)
add('swift', 'code', C.swift)
add('vue', 'code', C.vue)
add('svelte', 'code', C.svelte)
add('html htm xhtml', 'code', C.html)
add('css', 'code', C.css)
add('scss sass less', 'code', C.sass)
add('json jsonc json5', 'data', C.json)
add('yaml yml', 'config', C.yaml)
add('xml plist xsd xsl', 'code', C.xml)
add('sql', 'database', C.sql)
add('lua', 'code', C.lua)
add('dart', 'code', C.dart)
add('tf tfvars hcl', 'config', C.tf)
add('r rmd', 'code', C.r)
add('ex exs erl', 'code', C.ex)
add('hs', 'code', C.hs)
add('zig', 'code', C.zig)
add('nix', 'config', C.nix)
add('pl pm', 'code', C.text)
add('dockerfile', 'docker', C.docker)

// whole file names (lower case) that say more than their extension
const BY_NAME = {
  dockerfile: ['docker', C.docker],
  containerfile: ['docker', C.docker],
  'docker-compose.yml': ['docker', C.docker],
  'docker-compose.yaml': ['docker', C.docker],
  'compose.yml': ['docker', C.docker],
  'compose.yaml': ['docker', C.docker],
  '.dockerignore': ['docker', C.docker],
  '.gitignore': ['git', C.git],
  '.gitattributes': ['git', C.git],
  '.gitmodules': ['git', C.git],
  '.gitconfig': ['git', C.git],
  makefile: ['script', C.config],
  justfile: ['script', C.config],
  'cmakelists.txt': ['config', C.config],
  'package.json': ['data', '#cb3837'],
  'package-lock.json': ['lock', C.lock],
  'cargo.toml': ['config', C.rs],
  'cargo.lock': ['lock', C.lock],
  'go.mod': ['config', C.go],
  'go.sum': ['lock', C.lock],
  'requirements.txt': ['text', C.py],
  pipfile: ['config', C.py],
  'pyproject.toml': ['config', C.py],
  gemfile: ['config', C.rb],
  'yarn.lock': ['lock', C.lock],
  'pnpm-lock.yaml': ['lock', C.lock],
  'composer.json': ['data', C.php],
  license: ['text', C.text],
  'license.md': ['text', C.text],
  'license.txt': ['text', C.text],
  copying: ['text', C.text],
  authorized_keys: ['key', C.key],
  known_hosts: ['key', C.key],
  id_rsa: ['key', C.key],
  id_ecdsa: ['key', C.key],
  id_ed25519: ['key', C.key],
  id_dsa: ['key', C.key],
  '.bashrc': ['script', C.shell],
  '.bash_profile': ['script', C.shell],
  '.profile': ['script', C.shell],
  '.zshrc': ['script', C.shell],
  '.zprofile': ['script', C.shell],
  '.htaccess': ['config', C.config],
  '.npmrc': ['config', C.config],
  '.nvmrc': ['config', C.config],
  crontab: ['config', C.config],
}

// ---------- language and tool logos ----------
// Code and tooling files show the logo of their language or tool instead of the page.
// Brand colours that disappear on the dark theme (black) or the light one (pale yellow,
// white) are toned; the rest keep the brand colour.
const LOGO_COLOR = {
  rust: C.rs,
  openjdk: C.java,
  json: C.json,
  markdown: C.md,
  javascript: C.js,
  react: '#1fa8d1',
  c: C.c,
  cplusplus: '#5b8fd6',
  dotnet: C.cs,
  lua: C.lua,
  less: '#4a6fa5',
  haskell: C.hs,
  prisma: '#5a67d8',
  gradle: '#4d9fb0',
  cmake: '#3f7fc0',
  xml: C.xml,
  perl: '#3c96c8',
  bun: '#c9a26b',
  deno: C.text,
  dotenv: '#c9a400',
  editorconfig: C.text,
  css: '#7e57c2',
  prettier: '#d9a02a',
}

const LOGO_BY_EXT = {}
const logo = (exts, slug) => exts.split(' ').forEach((e) => (LOGO_BY_EXT[e] = slug))
logo('py pyw pyi', 'python')
logo('ipynb', 'jupyter')
logo('js mjs cjs', 'javascript')
logo('ts mts cts', 'typescript')
logo('jsx tsx', 'react')
logo('rs', 'rust')
logo('go', 'go')
logo('java jar class', 'openjdk')
logo('kt kts', 'kotlin')
logo('c h', 'c')
logo('cpp cc cxx hpp hh hxx', 'cplusplus')
logo('cs csproj sln vb', 'dotnet')
logo('fs fsx fsi', 'fsharp')
logo('php', 'php')
logo('rb erb gemspec rake', 'ruby')
logo('swift', 'swift')
logo('vue', 'vuedotjs')
logo('svelte', 'svelte')
logo('astro', 'astro')
logo('html htm xhtml', 'html5')
logo('css', 'css')
logo('scss sass', 'sass')
logo('less', 'less')
logo('json jsonc json5', 'json')
logo('yaml yml', 'yaml')
logo('md markdown mdx', 'markdown')
logo('lua', 'lua')
logo('dart', 'dart')
logo('tf tfvars hcl', 'terraform')
logo('r rmd', 'r')
logo('ex exs heex', 'elixir')
logo('erl hrl', 'erlang')
logo('hs lhs', 'haskell')
logo('zig', 'zig')
logo('nix', 'nixos')
logo('sh bash zsh ksh', 'gnubash')
logo('pl pm', 'perl')
logo('scala sc', 'scala')
logo('clj cljs cljc edn', 'clojure')
logo('jl', 'julia')
logo('ml mli', 'ocaml')
logo('toml', 'toml')
logo('xml xsd xsl plist', 'xml')
logo('graphql gql', 'graphql')
logo('prisma', 'prisma')
logo('wasm wat', 'webassembly')
logo('tex sty cls bib', 'latex')
logo('gradle', 'gradle')

// whole file names (lower case) → logo
const LOGO_BY_NAME = {
  dockerfile: 'docker',
  containerfile: 'docker',
  'docker-compose.yml': 'docker',
  'docker-compose.yaml': 'docker',
  'compose.yml': 'docker',
  'compose.yaml': 'docker',
  '.dockerignore': 'docker',
  '.gitignore': 'git',
  '.gitattributes': 'git',
  '.gitmodules': 'git',
  '.gitconfig': 'git',
  'package.json': 'npm',
  'package-lock.json': 'npm',
  '.npmrc': 'npm',
  'yarn.lock': 'yarn',
  '.yarnrc': 'yarn',
  '.yarnrc.yml': 'yarn',
  'pnpm-lock.yaml': 'pnpm',
  'pnpm-workspace.yaml': 'pnpm',
  'bun.lockb': 'bun',
  'bun.lock': 'bun',
  'bunfig.toml': 'bun',
  'deno.json': 'deno',
  'deno.jsonc': 'deno',
  '.nvmrc': 'nodedotjs',
  '.node-version': 'nodedotjs',
  'cargo.toml': 'rust',
  'cargo.lock': 'rust',
  'rust-toolchain.toml': 'rust',
  'go.mod': 'go',
  'go.sum': 'go',
  'go.work': 'go',
  'requirements.txt': 'python',
  'pyproject.toml': 'python',
  pipfile: 'python',
  'pipfile.lock': 'python',
  'setup.py': 'python',
  gemfile: 'ruby',
  'gemfile.lock': 'ruby',
  rakefile: 'ruby',
  'composer.json': 'php',
  'composer.lock': 'php',
  'pom.xml': 'apachemaven',
  'build.gradle': 'gradle',
  'build.gradle.kts': 'gradle',
  'settings.gradle': 'gradle',
  'cmakelists.txt': 'cmake',
  '.editorconfig': 'editorconfig',
  'nginx.conf': 'nginx',
  'ansible.cfg': 'ansible',
}

function logoFor(n) {
  if (LOGO_BY_NAME[n]) return LOGO_BY_NAME[n]
  if (n.startsWith('.env')) return 'dotenv'
  if (/^dockerfile[.-]/.test(n) || /\.dockerfile$/.test(n)) return 'docker'
  if (/^\.prettierrc|^prettier\.config\./.test(n)) return 'prettier'
  if (/^\.eslintrc|^eslint\.config\./.test(n)) return 'eslint'
  if (/^vite\.config\./.test(n)) return 'vite'
  if (/^tailwind\.config\./.test(n)) return 'tailwindcss'
  if (/^(kustomization|.*\.k8s)\.ya?ml$/.test(n)) return 'kubernetes'
  if (/\.d\.ts$/.test(n)) return 'typescript'
  return LOGO_BY_EXT[extOf(n)] || null
}

/** { kind, color, logo? } for a file name: `logo` is { title, path } for code and tooling. */
export function fileIcon(name) {
  const n0 = name.toLowerCase()
  const slug = logoFor(n0)
  if (slug && LANG_LOGOS[slug]) {
    const l = LANG_LOGOS[slug]
    return { kind: 'logo', color: LOGO_COLOR[slug] || l.hex, logo: { title: l.title, path: l.path } }
  }
  return pageIcon(name)
}

/** { kind, color } for files without a logo. */
function pageIcon(name) {
  const n = name.toLowerCase()
  const hit = BY_NAME[n]
  if (hit) return { kind: hit[0], color: hit[1] }
  if (n.startsWith('.env')) return { kind: 'config', color: C.key } // .env, .env.local… hold secrets
  if (/^readme(\.|$)/.test(n)) return { kind: 'text', color: C.md }
  if (/^dockerfile[.-]/.test(n)) return { kind: 'docker', color: C.docker }
  const byExt = BY_EXT[extOf(name)]
  if (byExt) return { kind: byExt[0], color: byExt[1] }
  return { kind: 'file', color: '' }
}
