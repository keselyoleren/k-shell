// iTerm2 inline images sent in pieces.
//
// imgcat (and iTerm2's other utilities) send an image as `1337;MultipartFile=<args>`, then
// many `1337;FilePart=<base64>` chunks and a final `1337;FileEnd`, so it gets through tmux.
// @xterm/addon-image 0.9 only reads the one-piece form `1337;File=<args>:<base64>`, so the
// pieces are collected here and handed to the addon's own iTerm2 handler as one image.

/** About 20 MB of image data once decoded: the addon's own size limit for an image. */
const MAX_BASE64 = 28_000_000

/**
 * Register the joiner on `term` for an ImageAddon already loaded on it. Register it after
 * the addon: xterm asks the newest handler first, and the addon would swallow the pieces.
 * Returns a disposable.
 */
export function joinMultipartImages(term, addon) {
  let head = null // the MultipartFile arguments while a transfer is going on
  let parts = []
  let length = 0
  const reset = () => {
    head = null
    parts = []
    length = 0
  }
  return term.parser.registerOscHandler(1337, (data) => {
    if (data.startsWith('MultipartFile=')) {
      reset()
      head = data.slice('MultipartFile='.length)
      return true
    }
    if (data.startsWith('FilePart=')) {
      if (head == null) return true
      length += data.length - 'FilePart='.length
      if (length > MAX_BASE64) reset()
      else parts.push(data.slice('FilePart='.length))
      return true
    }
    if (data === 'FileEnd') {
      if (head == null) return true
      const seq = `File=${head}:${parts.join('')}`
      reset()
      // not public API: the handler the addon registers for OSC 1337 (start / put / end)
      const iip = addon._handlers?.get('iip')
      if (!iip) return true
      const codes = new Uint32Array(seq.length)
      for (let i = 0; i < seq.length; i++) codes[i] = seq.charCodeAt(i)
      iip.start()
      iip.put(codes, 0, codes.length)
      return iip.end(true) // a promise while the image decodes: output waits for it, so order is kept
    }
    return false // File=… and anything else: the addon and other handlers
  })
}
