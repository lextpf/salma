import { useEffect, useMemo, useRef, useState } from 'react'
import { useParams, Link } from 'react-router-dom'
import { getFomod, listFomods } from './api'
import ConfidenceBreakdown from './components/ConfidenceBreakdown'
import ConfidencePill from './components/ConfidencePill'
import FomodStepCard from './components/FomodStepCard'
import Section from './components/Section'
import type { FomodDetail, RunDiagnostics } from './types'

const RETRY_DELAY_MS = 2000
const MAX_RETRIES = 3

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
        style={{ fontSize: 11, letterSpacing: '0.18em' }}
      >
        {label}
      </span>
      <span
        className="tabular-nums"
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 13,
          color: 'var(--ink-2)',
          letterSpacing: '0.04em',
        }}
      >
        {value}
      </span>
    </span>
  )
}

function DiagnosticsCard({ diagnostics }: { diagnostics: RunDiagnostics }) {
  const repro = diagnostics.repro
  const timings = diagnostics.timings_ms
  const groups = diagnostics.groups
  return (
    <div
      style={{
        background: 'var(--card)',
        border: '1px solid var(--rule)',
        borderRadius: 'var(--radius-md)',
        padding: '20px 24px',
      }}
    >
      <div className="flex items-center" style={{ gap: 16, marginBottom: 16, flexWrap: 'wrap' }}>
        <ConfidencePill confidence={diagnostics.confidence} size="lg" />
        <span style={{ width: 1, height: 14, background: 'var(--rule)' }} />
        <MetaChip label="Phase" value={diagnostics.phase_reached || 'n/a'} />
        <span style={{ width: 1, height: 14, background: 'var(--rule)' }} />
        <MetaChip label="Match" value={diagnostics.exact_match ? 'exact' : 'partial'} />
        <span style={{ width: 1, height: 14, background: 'var(--rule)' }} />
        <MetaChip label="Nodes" value={String(diagnostics.nodes_explored)} />
        {diagnostics.cache.hit && (
          <>
            <span style={{ width: 1, height: 14, background: 'var(--rule)' }} />
            <MetaChip label="Cache" value={diagnostics.cache.source || 'hit'} />
          </>
        )}
      </div>

      <ConfidenceBreakdown components={diagnostics.confidence.components} />

      <div
        className="flex"
        style={{
          gap: 24,
          marginTop: 18,
          paddingTop: 14,
          borderTop: '1px solid var(--rule-soft)',
          flexWrap: 'wrap',
        }}
      >
        <div className="flex flex-col" style={{ gap: 4, minWidth: 200 }}>
          <span className="ui-label" style={{ fontSize: 10, letterSpacing: '0.18em' }}>
            Reproduction
          </span>
          <div className="flex" style={{ gap: 12, flexWrap: 'wrap' }}>
            <MetaChip label="Repr" value={String(repro.reproduced)} />
            <MetaChip label="Miss" value={String(repro.missing)} />
            <MetaChip label="Extra" value={String(repro.extra)} />
            <MetaChip label="SizeMM" value={String(repro.size_mismatch)} />
            <MetaChip label="HashMM" value={String(repro.hash_mismatch)} />
          </div>
        </div>
        <div className="flex flex-col" style={{ gap: 4, minWidth: 200 }}>
          <span className="ui-label" style={{ fontSize: 10, letterSpacing: '0.18em' }}>
            Groups
          </span>
          <div className="flex" style={{ gap: 12, flexWrap: 'wrap' }}>
            <MetaChip label="Total" value={String(groups.total)} />
            <MetaChip label="Prop" value={String(groups.resolved_by_propagation)} />
            <MetaChip label="CSP" value={String(groups.resolved_by_csp)} />
          </div>
        </div>
        <div className="flex flex-col" style={{ gap: 4, minWidth: 220 }}>
          <span className="ui-label" style={{ fontSize: 10, letterSpacing: '0.18em' }}>
            Timings (ms)
          </span>
          <div className="flex" style={{ gap: 12, flexWrap: 'wrap' }}>
            <MetaChip label="List" value={String(timings.list)} />
            <MetaChip label="Scan" value={String(timings.scan)} />
            <MetaChip label="Solve" value={String(timings.solve)} />
            <MetaChip label="Total" value={String(timings.total)} />
          </div>
        </div>
      </div>
    </div>
  )
}

export default function FomodDetailPage() {
  const { name } = useParams<{ name: string }>()
  const [data, setData] = useState<FomodDetail | null>(null)
  const [loadedAt, setLoadedAt] = useState<number | null>(null)
  const [metaSize, setMetaSize] = useState<number | null>(null)
  const [metaModified, setMetaModified] = useState<number | null>(null)
  const [loadError, setLoadError] = useState<string | null>(null)
  const retryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const retryCountRef = useRef(0)
  const loadFomodRef = useRef<() => void>(() => {})

  useEffect(() => {
    if (!name) return
    const decoded = decodeURIComponent(name)
    setData(null)
    setLoadError(null)
    retryCountRef.current = 0
    const abortController = new AbortController()

    const clearRetryTimer = () => {
      if (retryTimerRef.current) {
        clearTimeout(retryTimerRef.current)
        retryTimerRef.current = null
      }
    }

    const loadFomod = () => {
      setLoadError(null)
      getFomod(decoded)
        .then(d => {
          if (abortController.signal.aborted) return
          setData(d)
          setLoadedAt(Date.now())
          retryCountRef.current = 0
          clearRetryTimer()
        })
        .catch(e => {
          if (abortController.signal.aborted) return
          retryCountRef.current++
          const msg = e instanceof Error ? e.message : 'Failed to load FOMOD'
          if (retryCountRef.current >= MAX_RETRIES) {
            console.warn(`[fomod-detail] gave up loading "${decoded}" after ${MAX_RETRIES} attempts`, e)
            setLoadError(msg)
            return
          }
          console.warn(`[fomod-detail] failed to load "${decoded}", retrying (${retryCountRef.current}/${MAX_RETRIES})`, e)
          if (!retryTimerRef.current) {
            retryTimerRef.current = setTimeout(() => {
              retryTimerRef.current = null
              loadFomod()
            }, RETRY_DELAY_MS)
          }
        })
    }

    loadFomodRef.current = loadFomod
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
        className="flex items-center serif-link-arrow serif-link-arrow-back"
        style={{
          gap: 8,
          fontFamily: 'var(--font-mono)',
          fontSize: 12,
          letterSpacing: '0.12em',
          textTransform: 'uppercase',
          marginBottom: 16,
          width: 'fit-content',
          flexShrink: 0,
        }}
      >
        <span className="arrow" aria-hidden="true">
          <i className="fa-duotone fa-solid fa-arrow-left-long" style={{ fontSize: 13 }} />
        </span>
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
            02 -
          </span>
          <span>Sec. Inspection - {decodedName}</span>
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
      {loadError ? (
        <div
          className="reveal reveal-delay-4"
          style={{
            maxWidth: 620,
            padding: '20px 24px',
            background: 'var(--paper-3)',
            border: '1px solid var(--rule)',
            borderRadius: 'var(--radius-md)',
          }}
        >
          <div className="flex items-center" style={{ gap: 10, marginBottom: 8 }}>
            <span className="dot-status dot-status-error" />
            <span
              className="ui-label"
              style={{ color: 'var(--accent)', fontSize: 10 }}
            >
              Failed to load
            </span>
          </div>
          <p
            className="display-serif-italic"
            style={{ fontSize: 16, color: 'var(--ink)', marginBottom: 12 }}
          >
            {loadError}
          </p>
          <div className="flex" style={{ gap: 8 }}>
            <button
              type="button"
              className="tool-btn"
              onClick={() => {
                retryCountRef.current = 0
                setLoadError(null)
                loadFomodRef.current()
              }}
            >
              <i className="fa-duotone fa-solid fa-arrows-spin" style={{ fontSize: 12 }} />
              <span>Retry</span>
            </button>
            <Link
              to="/fomods"
              className="tool-btn"
              style={{ textDecoration: 'none' }}
            >
              <i className="fa-duotone fa-solid fa-arrow-left-long" style={{ fontSize: 12 }} />
              <span>Back to library</span>
            </Link>
          </div>
        </div>
      ) : !data ? (
        <div className="reveal reveal-delay-4">
          <Section n="01" label="Steps" title="Loading..." corner="01">
            <div className="flex flex-col" style={{ gap: 12 }}>
              {[0, 1, 2].map(i => (
                <div
                  key={i}
                  style={{
                    padding: '20px 24px',
                    border: '1px solid var(--rule)',
                    borderRadius: 'var(--radius-md)',
                    background: 'var(--card)',
                  }}
                >
                  <div className="flex items-center" style={{ gap: 12, marginBottom: 14 }}>
                    <div className="skeleton-line" style={{ height: 18, width: 32 }} />
                    <div className="skeleton-line" style={{ width: 18, height: 1 }} />
                    <div className="skeleton-line" style={{ height: 18, width: ['180px', '220px', '160px'][i] }} />
                  </div>
                  <div className="flex flex-col" style={{ gap: 6 }}>
                    <div className="skeleton-line" style={{ height: 13, width: '70%' }} />
                    <div className="skeleton-line" style={{ height: 13, width: '55%' }} />
                    <div className="skeleton-line" style={{ height: 13, width: '62%' }} />
                  </div>
                </div>
              ))}
            </div>
          </Section>
        </div>
      ) : (
        <>
          {/* Diagnostics (only when schema v2 inference produced a diagnostics block) */}
          {data.diagnostics && (
            <div className="reveal reveal-delay-4" style={{ marginBottom: 16 }}>
              <Section
                n="00"
                label="Diagnostics"
                title="Inference confidence"
                corner="00"
              >
                <DiagnosticsCard diagnostics={data.diagnostics} />
              </Section>
            </div>
          )}

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
                  // {JSON.stringify(data).length.toLocaleString()} bytes - click expand to view
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
