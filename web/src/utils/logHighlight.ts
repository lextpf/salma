export interface HighlightSegment {
  text: string
  cls: string
}

// Pre-pass: extract quoted strings so the main regex doesn't have to deal with them.
// Handles apostrophes inside single-quoted strings (e.g. 'JK's Temple of Talos').
export function extractQuoted(text: string): HighlightSegment[] {
  const out: HighlightSegment[] = []
  let i = 0
  let plain = 0

  while (i < text.length) {
    if (text[i] === '"') {
      if (i > plain) out.push({ text: text.slice(plain, i), cls: '' })
      let end = i + 1
      while (end < text.length) {
        if (text[end] === '\\') { end += 2; continue }
        if (text[end] === '"') { end++; break }
        end++
      }
      out.push({ text: text.slice(i, end), cls: 'log-string' })
      i = end; plain = end
    } else if (text[i] === "'") {
      // Closing ' must NOT be followed by an alphanumeric char (that would be an apostrophe)
      let end = i + 1
      let found = false
      while (end < text.length) {
        if (text[end] === "'") {
          const next = end + 1 < text.length ? text[end + 1] : ''
          if (!next || !/[a-zA-Z0-9]/.test(next)) { end++; found = true; break }
        }
        end++
      }
      if (found && end - i > 2) {
        if (i > plain) out.push({ text: text.slice(plain, i), cls: '' })
        out.push({ text: text.slice(i, end), cls: 'log-string' })
        i = end; plain = end
      } else {
        i++
      }
    } else {
      i++
    }
  }
  if (plain < text.length) out.push({ text: text.slice(plain), cls: '' })
  return out
}

const TOKEN_REGEX = new RegExp([
  /(\[[^\]]+\])/,                                          // [tags]
  /(https?:\/\/[^\s,;)"']+)/,                             // URLs
  /([A-Z]:(?:[\\/][^\\/:*?"<>|\r\n,]*[a-zA-Z0-9._()])+|\/(?:usr|var|home|etc|tmp|mnt|opt)\/[^\s,;]+)/, // file paths
  /(HTTP\/\d(?:\.\d)?)/,                                  // HTTP version
  /\b(GET|POST|PUT|DELETE|PATCH|HEAD|OPTIONS)\b/,          // HTTP methods
  /(\/api\/[^\s,;)"']+)/,                                 // API routes
  /\b(\d+\/\d+)\b/,                                        // step counters (0/9, 335/335)
  /\b(PASS|INFERRED|FAIL|SKIP|ERROR|DONE|Passed|Failed)\b/, // test/scan results + summary keywords
  /((?:HTTP(?:\/\d(?:\.\d)?)?\s+[1-5]\d{2}\b)|(?:[Ss][Tt][Aa][Tt][Uu][Ss](?:\s*[Cc][Oo][Dd][Ee])?\s*[:=]\s*[1-5]\d{2}\b)|(?:\b[1-5]\d{2}\b(?=\s+\d+\s*$)))/, // HTTP status
  /(\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}(?::\d+)?)/,       // IP addresses
  /\b([0-9A-F]{10,})\b/,                                  // hex IDs
  /([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})/i, // GUIDs
  /\b(\d+(?:\.\d+)?(?:ms|s|m|h))\b/,                       // durations
  /\b(\d+(?:\.\d+)?\s*(?:KB|MB|GB|bytes|B|%|files?|folders?|chars|plugins?|groups?|nodes?|steps?|components?))\b/, // numbers with units
  /\b(\w+)(?==(?:\d|true\b|false\b))/,                        // key in key=value pairs (entries=18, exact=true)
  /\b(true|false)\b/,                                        // boolean values
  /\b(\d+(?:\.\d+)?)(?=[kMG]\b|\b)/,                         // plain numbers (stops before SI suffix)
  /(=>|->|::|!=|<=|>=|&&|\|\||=+)/,                          // operators
].map(r => r.source).join('|'), 'g')

export function highlightTokens(text: string, parts: HighlightSegment[], depth = 0) {
  if (depth > 3) { parts.push({ text, cls: '' }); return }
  const regex = new RegExp(TOKEN_REGEX.source, TOKEN_REGEX.flags)
  let lastIndex = 0
  let match: RegExpExecArray | null
  while ((match = regex.exec(text)) !== null) {
    if (match.index > lastIndex) {
      parts.push({ text: text.slice(lastIndex, match.index), cls: '' })
    }
    const [full, tag, url, path, protocol, method, route, stepCounter, testResult, statusCtx, ip, reqId, guid, duration, unitNum, kvKey, boolVal, num, op] = match
    if (tag) parts.push({ text: full, cls: 'log-tag' })
    else if (url) parts.push({ text: full, cls: 'log-url' })
    else if (path) {
      // Trim trailing non-path text after a file extension (e.g. ".dds (priority: 0)")
      const cleaned = full.replace(/(\.\w{1,10})\s(?!.*[\\\/]).*$/, '$1')
      if (cleaned.length < full.length) {
        parts.push({ text: cleaned, cls: 'log-path' })
        highlightTokens(full.slice(cleaned.length), parts, depth + 1)
      } else {
        parts.push({ text: full, cls: 'log-path' })
      }
    }
    else if (protocol) parts.push({ text: full, cls: 'log-protocol' })
    else if (method) parts.push({ text: full, cls: 'log-method' })
    else if (route) parts.push({ text: full, cls: 'log-route' })
    else if (stepCounter) parts.push({ text: full, cls: 'log-number' })
    else if (testResult) {
      const cls = testResult === 'PASS' || testResult === 'Passed' || testResult === 'INFERRED' || testResult === 'DONE' ? 'log-status-ok'
        : testResult === 'FAIL' || testResult === 'ERROR' || testResult === 'Failed' ? 'log-status-err'
        : 'log-level-info' // SKIP
      parts.push({ text: full, cls })
    }
    else if (statusCtx) {
      const code = parseInt((statusCtx.match(/[1-5]\d{2}/) || ['0'])[0], 10)
      parts.push({ text: full, cls: code >= 400 ? 'log-status-err' : code >= 200 && code < 400 ? 'log-status-ok' : 'log-number' })
    }
    else if (ip) parts.push({ text: full, cls: 'log-ip' })
    else if (reqId) parts.push({ text: full, cls: 'log-ip' })
    else if (guid) parts.push({ text: full, cls: 'log-guid' })
    else if (duration) parts.push({ text: full, cls: 'log-duration' })
    else if (unitNum) parts.push({ text: full, cls: 'log-storage' })
    else if (kvKey) parts.push({ text: full, cls: 'log-key' })
    else if (boolVal) parts.push({ text: full, cls: 'log-number' })
    else if (num) parts.push({ text: full, cls: 'log-number' })
    else if (op) parts.push({ text: full, cls: 'log-operator' })
    else parts.push({ text: full, cls: '' })
    lastIndex = match.index + full.length
  }
  if (lastIndex < text.length) {
    parts.push({ text: text.slice(lastIndex), cls: '' })
  }
}

export function highlightLog(line: string): HighlightSegment[] {
  const parts: HighlightSegment[] = []
  let remaining = line

  // Match timestamp at start:
  // 2024-01-15 12:34:56(.ms), [2024-01-15 12:34:56], or 3-01 18:37:31.708
  const tsMatch = remaining.match(/^(\[?(?:\d{4}-\d{2}-\d{2}|\d{1,2}-\d{2})[\sT]\d{2}:\d{2}:\d{2}(?:\.\d+)?\]?\s*)/)
  if (tsMatch) {
    parts.push({ text: tsMatch[1], cls: 'log-timestamp' })
    remaining = remaining.slice(tsMatch[1].length)
  }

  // Also match short timestamps like HH:MM:SS(.ms) (test.log format)
  if (!tsMatch) {
    const shortTsMatch = remaining.match(/^(\d{2}:\d{2}:\d{2}(?:\.\d+)?\s+)/)
    if (shortTsMatch) {
      parts.push({ text: shortTsMatch[1], cls: 'log-timestamp' })
      remaining = remaining.slice(shortTsMatch[1].length)
    }
  }

  // Match log level
  const levelMatch = remaining.match(/^(?:-\s*)?(ERROR|WARNING|WARN|INFO|DEBUG|TRACE|CRITICAL|FATAL)\b(?:\s*-(?!-)\s*)?/i)
  if (levelMatch) {
    const level = levelMatch[1].toUpperCase()
    const cls = level === 'ERROR' || level === 'CRITICAL' || level === 'FATAL' ? 'log-level-error'
      : level === 'WARNING' || level === 'WARN' ? 'log-level-warning'
      : level === 'DEBUG' || level === 'TRACE' ? 'log-level-debug'
      : 'log-level-info'
    parts.push({ text: levelMatch[0], cls })
    remaining = remaining.slice(levelMatch[0].length)
  }

  // Pre-pass: extract quoted strings, then highlight unquoted segments with the token regex
  if (remaining) {
    for (const seg of extractQuoted(remaining)) {
      if (seg.cls) parts.push(seg)
      else highlightTokens(seg.text, parts)
    }
  }

  return parts
}
