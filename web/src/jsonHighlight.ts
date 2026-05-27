// JSON tokenizer for the inspector's JSON tab. Splits a pretty-printed JSON
// string into spans tagged with the .json-* CSS classes from index.css, so the
// raw record can be colorized without a highlighter dependency.

export interface JsonToken {
  text: string
  cls: string
}

export function highlightJson(json: string): JsonToken[] {
  const regex =
    /("(?:\\.|[^"\\])*"\s*:)|("(?:\\.|[^"\\])*")|([-+]?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)|(\btrue\b|\bfalse\b)|(\bnull\b)|([{}[\],])/g
  const parts: JsonToken[] = []
  let lastIndex = 0
  let match: RegExpExecArray | null
  while ((match = regex.exec(json)) !== null) {
    if (match.index > lastIndex) {
      parts.push({ text: json.slice(lastIndex, match.index), cls: '' })
    }
    if (match[1]) parts.push({ text: match[1], cls: 'json-key' })
    else if (match[2]) parts.push({ text: match[2], cls: 'json-string' })
    else if (match[3]) parts.push({ text: match[3], cls: 'json-number' })
    else if (match[4]) parts.push({ text: match[4], cls: 'json-boolean' })
    else if (match[5]) parts.push({ text: match[5], cls: 'json-null' })
    else if (match[6]) parts.push({ text: match[6], cls: 'json-bracket' })
    lastIndex = match.index + match[0].length
  }
  if (lastIndex < json.length) {
    parts.push({ text: json.slice(lastIndex), cls: '' })
  }
  return parts
}
