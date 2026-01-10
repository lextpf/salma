import { useEffect, useMemo, useRef, useState } from 'react'
import { useParams, Link } from 'react-router-dom'
import { getFomod, listFomods } from '../api'
import FomodStepCard from '../components/FomodStepCard'
import Section from '../components/Section'
import type { FomodDetail } from '../types'

const RETRY_DELAY_MS = 2000

function highlightJson(json: string) {
  const regex = /("(?:\\.|[^"\\])*"\s*:)|("(?:\\.|[^"\\])*")|([-+]?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)|(\btrue\b|\bfalse\b)|(\bnull\b)|([{}[\],])/g
  const parts: { text: string; cls: string }[] = []
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
  if (lastIndex < json.length) parts.push({ text: json.slice(lastIndex), cls: '' })
  return parts
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} b`
  if (bytes < 1048576) return `${(bytes / 1024).toFixed(1)} kb`
  return `${(bytes / 1048576).toFixed(1)} mb`
}

function MetaChip({ label, value }: { label: string; value: string }) {
  return (
    <span className="flex items-center" style={{ gap: 6 }}>
      <span
        className="ui-label"
        style={{ fontSize: 9.5, letterSpacing: '0.18em' }}
      >
        {label}
      </span>
      <span
        className="tabular-nums"
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 11.5,
          color: 'var(--ink-2)',
          letterSpacing: '0.04em',
        }}
      >
        {value}
      </span>
    </span>
  )
}

export default function FomodDetailPage() {
  const { name } = useParams<{ name: string }>()
  const [data, setData] = useState<FomodDetail | null>(null)
  const [loadedAt, setLoadedAt] = useState<number | null>(null)
  const [metaSize, setMetaSize] = useState<number | null>(null)
  const [metaModified, setMetaModified] = useState<number | null>(null)
  const retryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const inFlightRef = useRef(false)

  useEffect(() => {
    if (!name) return
    const decoded = decodeURIComponent(name)
    setData(null)
    const abortController = new AbortController()

    const clearRetryTimer = () => {
      if (retryTimerRef.current) {
        clearTimeout(retryTimerRef.current)
        retryTimerRef.current = null
      }
    }

    const loadFomod = (force = false) => {
      if (inFlightRef.current && !force) return
      inFlightRef.current = true
      getFomod(decoded)
        .then(d => {
          if (abortController.signal.aborted) return
          setData(d)
          setLoadedAt(Date.now())
          clearRetryTimer()
        })
        .catch(e => {
          if (abortController.signal.aborted) return
          console.warn(`[fomod-detail] failed to load "${decoded}", retrying`, e)
          if (!retryTimerRef.current) {
            retryTimerRef.current = setTimeout(() => {
              retryTimerRef.current = null
              loadFomod(true)
            }, RETRY_DELAY_MS)
          }
        })
        .finally(() => {
          inFlightRef.current = false
        })
    }

    loadFomod()

    listFomods()
      .then(items => {
        if (abortController.signal.aborted) return
        const item = items.find(f => f.name === decoded)
        if (item) { setMetaSize(item.size); setMetaModified(item.modified) }
      })
      .catch(e => {
        if (abortController.signal.aborted) return
        console.warn('[fomod-detail] failed to load metadata list', e)
      })

    return () => {
      abortController.abort()
      clearRetryTimer()
    }
  }, [name])

  const decodedName = name ? decodeURIComponent(name) : 'Unknown'
  const updatedRaw = data?.updated ?? data?.modified ?? metaModified
  const updatedText = typeof updatedRaw === 'number'
    ? new Date(updatedRaw).toLocaleString(undefined, { year: 'numeric', month: 'short', day: '2-digit', hour: '2-digit', minute: '2-digit' })
    : loadedAt
      ? new Date(loadedAt).toLocaleTimeString()
      : 'unknown'
  const sizeText = typeof metaSize === 'number' ? formatSize(metaSize) : 'n/a'
  const modName = data?.moduleName || decodedName
  const steps = data?.steps ?? []
  const highlightedJson = useMemo(
    () => data ? highlightJson(JSON.stringify(data, null, 2)) : null,
    [data]
  )
  const [showJson, setShowJson] = useState(false)

  return (
    <div className="page-fill">
      {/* Back link */}
      <Link
        to="/fomods"
        className="flex items-center"
        style={{
          gap: 8,
          fontFamily: 'var(--font-mono)',
          fontSize: 10.5,
          letterSpacing: '0.12em',
          textTransform: 'uppercase',
          color: 'var(--ink-3)',
          textDecoration: 'none',
          marginBottom: 16,
          width: 'fit-content',
          flexShrink: 0,
        }}
      >
        <i className="fa-duotone fa-solid fa-arrow-left-long" style={{ fontSize: 11 }} />
        // back to library
      </Link>

      {/* Header */}
      <header style={{ marginBottom: 24, flexShrink: 0 }}>
        <p
          className="reveal reveal-delay-1"
          style={{
            display: 'inline-flex',
            alignItems: 'center',
            gap: 10,
            fontFamily: 'var(--font-mono)',
            fontSize: 11,
            letterSpacing: '0.18em',
            textTransform: 'uppercase',
            color: 'var(--ink-3)',
            marginBottom: 18,
          }}
        >
          <span
            className="display-serif-italic"
            style={{ fontSize: 12, color: 'var(--accent)', textTransform: 'none' }}
          >
            02 ·
          </span>
          <span>§ Inspection · {decodedName}</span>
        </p>

        <h1
          className="display-serif reveal reveal-delay-2"
          style={{
            fontSize: 56,
            lineHeight: 0.95,
            color: 'var(--ink)',
            margin: 0,
            wordBreak: 'break-word',
          }}
        >
          {modName}
          <span style={{ color: 'var(--accent)' }}>.</span>
        </h1>

        <div
          className="flex items-center reveal reveal-delay-3"
          style={{
            marginTop: 14,
            gap: 24,
            flexWrap: 'wrap',
          }}
        >
          <MetaChip label="Steps" value={String(steps.length)} />
          <span style={{ width: 1, height: 14, background: 'var(--rule)' }} />
          <MetaChip label="Size" value={sizeText} />
          <span style={{ width: 1, height: 14, background: 'var(--rule)' }} />
          <MetaChip label="Updated" value={updatedText} />
        </div>
      </header>

      <div className="scroll-pane" style={{ flex: 1, minHeight: 0, overflow: 'auto' }}>
      {!data ? (
        <div className="reveal reveal-delay-4">
          <Section n="01" label="Steps" title="Loading..." corner="01">
            <div className="skeleton-line" style={{ height: 14, width: 220, marginBottom: 12 }} />
            <div className="skeleton-line" style={{ height: 12, width: 160, marginBottom: 8 }} />
            <div className="skeleton-line" style={{ height: 12, width: 200 }} />
          </Section>
        </div>
      ) : (
        <>
          {/* Steps */}
          <div className="reveal reveal-delay-4" style={{ marginBottom: 16 }}>
            <Section
              n="01"
              label="Steps"
              title="Installation steps"
              corner="01"
              meta={
                <span className="ui-micro tabular-nums">
                  {steps.length} {steps.length === 1 ? 'step' : 'steps'}
                </span>
              }
            >
              {steps.length === 0 ? (
                <p className="timestamp-print">// no steps in this FOMOD configuration</p>
              ) : (
                <div className="flex flex-col" style={{ gap: 12 }}>
                  {steps.map((step, i) => (
                    <FomodStepCard key={step.name || `step-${i}`} step={step} index={i} />
                  ))}
                </div>
              )}
            </Section>
          </div>

          {/* Raw JSON */}
          <div className="reveal reveal-delay-5">
            <Section
              n="02"
              label="JSON"
              title="Raw configuration"
              corner="02"
              bodyPadding="none"
              meta={
                <button
                  type="button"
                  onClick={() => setShowJson(v => !v)}
                  className="tool-btn"
                  style={{ padding: '6px 10px', fontSize: 10 }}
                >
                  <i
                    className={`fa-duotone fa-solid ${showJson ? 'fa-caret-down' : 'fa-caret-right'}`}
                    style={{ fontSize: 11 }}
                  />
                  <span
                    style={{
                      fontFamily: 'var(--font-mono)',
                      letterSpacing: '0.14em',
                      textTransform: 'uppercase',
                      color: 'var(--ink-3)',
                    }}
                  >
                    {showJson ? 'Collapse' : 'Expand'}
                  </span>
                </button>
              }
            >
              {showJson ? (
                <div
                  style={{
                    maxHeight: '55vh',
                    overflow: 'auto',
                    background: 'var(--paper-2)',
                    borderTop: '1px solid var(--rule-soft)',
                  }}
                  className="scroll-pane"
                >
                  <pre
                    className="log-viewer"
                    style={{
                      margin: 0,
                      padding: '20px 28px',
                      fontSize: 12,
                      lineHeight: 1.7,
                    }}
                  >
                    {highlightedJson?.map((p, i) => (
                      p.cls
                        ? <span key={i} className={p.cls}>{p.text}</span>
                        : <span key={i}>{p.text}</span>
                    ))}
                  </pre>
                </div>
              ) : (
                <p
                  className="timestamp-print"
                  style={{ padding: '16px 28px' }}
                >
                  // {JSON.stringify(data).length.toLocaleString()} bytes — click expand to view
                </p>
              )}
            </Section>
          </div>
        </>
      )}
      </div>
    </div>
  )
}
