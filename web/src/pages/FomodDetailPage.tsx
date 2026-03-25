import { useEffect, useMemo, useRef, useState } from 'react'
import { useParams, Link } from 'react-router-dom'
import { getFomod, listFomods } from '../api'
import FomodStepCard from '../components/FomodStepCard'
import type { FomodDetail } from '../types'

const RETRY_DELAY_MS = 2000

function highlightJson(json: string) {
  // Tokenize JSON string into colored spans
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
  if (lastIndex < json.length) {
    parts.push({ text: json.slice(lastIndex), cls: '' })
  }
  return parts
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1048576) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / 1048576).toFixed(1)} MB`
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
        if (item) {
          setMetaSize(item.size)
          setMetaModified(item.modified)
        }
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
    ? new Date(updatedRaw).toLocaleString()
    : loadedAt
      ? new Date(loadedAt).toLocaleTimeString()
      : 'Unknown'
  const sizeText = typeof metaSize === 'number' ? formatSize(metaSize) : 'N/A'

  const modName = data?.moduleName || decodedName
  const steps = data?.steps ?? []

  const highlightedJson = useMemo(
    () => data ? highlightJson(JSON.stringify(data, null, 2)) : null,
    [data]
  )

  return (
    <div className="fomods-page animate-fade-in h-[calc(100vh-4rem)] flex flex-col overflow-hidden">
      <Link to="/fomods" className="inline-flex items-center gap-2 text-xs text-on-surface-variant hover:text-primary transition-colors mb-6 shrink-0">
        <i className="fa-duotone fa-solid fa-arrow-left text-[0.6rem]" />Back to FOMODs
      </Link>

      <header className="page-header page-header-fomods mb-5 shrink-0">
        <h1 className="page-title text-xl font-bold text-on-surface flex items-center gap-3">
          <span className="inline-flex w-6 shrink-0 items-center justify-center">
            <i className="fa-duotone fa-solid fa-box-archive icon-gradient icon-gradient-spring text-lg" />
          </span>
          {modName}
        </h1>
        <div className="mt-1 ml-9 w-[calc(100%-2.25rem)] min-w-0 flex flex-nowrap items-center gap-1.5 text-on-surface-variant">
          <span className="rounded-full pl-0 pr-2.5 py-1 inline-flex items-center gap-1.5 hover-intent shrink-0">
            <i className="fa-duotone fa-solid fa-layer-group icon-gradient icon-gradient-forest icon-sm" />
            <span className="ui-micro uppercase tracking-[0.08em]">Steps</span>
            <span className="text-[0.72rem] text-on-surface tabular-nums">{steps.length}</span>
          </span>
          <span className="rounded-full px-2.5 py-1 inline-flex items-center gap-1.5 hover-intent shrink-0">
            <i className="fa-duotone fa-solid fa-weight-hanging icon-gradient icon-gradient-copper icon-sm" />
            <span className="ui-micro uppercase tracking-[0.08em]">Size</span>
            <span className="text-[0.72rem] text-on-surface tabular-nums">{sizeText}</span>
          </span>
          <span className="rounded-full px-2.5 py-1 inline-flex items-center gap-1.5 hover-intent shrink-0">
            <i className="fa-duotone fa-solid fa-clock icon-gradient icon-gradient-steel icon-sm" />
            <span className="ui-micro uppercase tracking-[0.08em]">Updated</span>
            <span className="text-[0.72rem] text-on-surface" title={updatedText}>{updatedText}</span>
          </span>
        </div>
      </header>

      {!data ? (
        <div className="flex-1 min-h-0">
          <div className="rounded-xl border border-outline-variant/30 bg-surface-container/35 p-4 mb-3">
            <div className="skeleton-line h-4 w-56 mb-2" />
            <div className="skeleton-line h-3 w-28" />
          </div>
          <div className="rounded-xl border border-outline-variant/30 bg-surface-container/35 p-4">
            <div className="skeleton-line h-3 w-44 mb-2" />
            <div className="skeleton-line h-3 w-64 mb-2" />
            <div className="skeleton-line h-3 w-48" />
          </div>
        </div>
      ) : (
        <>
          {/* Raw JSON */}
          <details className="mb-5 group shrink-0">
            <summary className="cursor-pointer text-xs text-on-surface-variant hover:text-on-surface transition-colors
                           flex items-center gap-2 select-none">
              <i className="fa-duotone fa-solid fa-code icon-gradient icon-gradient-nebula text-[0.65rem]" />
              View raw JSON
              <i className="fa-duotone fa-solid fa-chevron-right text-[0.55rem] text-outline transition-transform duration-200 group-open:rotate-90" />
            </summary>
            <div className="mt-3 rounded-xl aurora-glow">
              <div className="relative rounded-xl panel-modern overflow-hidden py-2.5 px-1.5">
                <div className="max-h-[55vh] overflow-auto scroll-pane">
                  <pre className="m-0 p-4 text-xs log-viewer">
                    {highlightedJson?.map((p, i) => (
                      p.cls
                        ? <span key={i} className={p.cls}>{p.text}</span>
                        : <span key={i}>{p.text}</span>
                    ))}
                  </pre>
                </div>
              </div>
            </div>
          </details>

          {steps.length === 0 ? (
            <div className="flex-1 min-h-0 py-10">
              <div className="empty-state-card">
                <div className="w-9 h-9 rounded-xl bg-surface-container-high/60 border border-outline-variant/25 flex items-center justify-center shadow-elevation-1">
                  <i className="fa-duotone fa-solid fa-circle-info icon-gradient icon-gradient-steel icon-sm" />
                </div>
                <p className="text-sm text-on-surface-variant">No steps in this FOMOD configuration.</p>
              </div>
            </div>
          ) : (
            <div className="flex-1 min-h-0 overflow-y-auto scroll-pane px-3">
              <div className="flex flex-col gap-3 pb-2">
                {steps.map((step, i) => (
                  <FomodStepCard key={step.name || `step-${i}`} step={step} index={i} />
                ))}
              </div>
            </div>
          )}
        </>
      )}
    </div>
  )
}
