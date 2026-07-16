import { useCallback, useEffect, useRef, useState } from 'react'
import { useNavigate, useParams } from 'react-router-dom'
import { listFomods } from './api'
import { useScanJob } from './useScanJob'
import { useSystemStatus } from './useSystemStatus'
import { useRecordDetail } from './useRecordDetail'
import Kicker from './comps/Kicker'
import MIcon from './comps/MIcon'
import VfsTree from './comps/library/VfsTree'
import RecordsList from './comps/library/RecordsList'
import Inspector from './comps/library/Inspector'
import type { FomodEntry } from './types'

const RETRY_DELAY_MS = 2000

// Module 02 - Library. The selection-driven triptych: a VFS tree of the selected
// record (left), the priority-ordered records list (middle), and the record
// inspector (right). Selection lives in the URL (/fomods/:name); the detail fetch
// is lifted here so the tree and inspector share one load. Search, scan, and tab
// state are local; VfsTree + Inspector remount on record change (via key) so
// their internal state (collapsed folders, active tab) resets cleanly.
export default function LibraryPage() {
  const { name } = useParams<{ name: string }>()
  const navigate = useNavigate()
  const selectedName = name ? decodeURIComponent(name) : null

  const [fomods, setFomods] = useState<FomodEntry[]>([])
  const [listLoading, setListLoading] = useState(true)
  const [search, setSearch] = useState('')
  const retryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const inFlightRef = useRef(false)
  // Initialized to a noop so loadList can reference it before the latest-ref
  // effect wires it up (avoids a use-before-declaration cycle).
  const loadListRef = useRef<(force?: boolean) => void>(() => {})

  const clearRetryTimer = useCallback(() => {
    if (retryTimerRef.current) {
      clearTimeout(retryTimerRef.current)
      retryTimerRef.current = null
    }
  }, [])

  const loadList = useCallback(
    (force = false) => {
      if (inFlightRef.current && !force) return
      // Note: no synchronous setListLoading(true) here. The initial value is
      // already true, and background refreshes (scan, retry) keep the current
      // list visible rather than flashing a skeleton. The .catch path re-arms
      // the loading flag asynchronously while retries are pending.
      inFlightRef.current = true
      listFomods()
        .then(data => {
          setFomods(data)
          setListLoading(false)
          clearRetryTimer()
        })
        .catch(e => {
          console.warn('[library] failed to load list, retrying', e)
          setListLoading(true)
          if (!retryTimerRef.current) {
            retryTimerRef.current = setTimeout(() => {
              retryTimerRef.current = null
              loadListRef.current(true)
            }, RETRY_DELAY_MS)
          }
        })
        .finally(() => {
          inFlightRef.current = false
        })
    },
    [clearRetryTimer],
  )

  // Latest-ref pattern: lets the retry timer call the current loadList without
  // re-subscribing consumers (loadList itself stays referentially stable).
  useEffect(() => {
    loadListRef.current = loadList
  })

  useEffect(() => {
    loadList()
    return clearRetryTimer
  }, [loadList, clearRetryTimer])

  const { status } = useSystemStatus()
  const pluginInstalled = status?.pluginInstalled === true

  const onScanComplete = useCallback(
    (success: boolean) => {
      if (success) loadList(true)
    },
    [loadList],
  )
  const { scanRunning, handleScanFomods } = useScanJob(pluginInstalled, onScanComplete)

  const { detail, error: detailError, retry: retryDetail } = useRecordDetail(selectedName)

  const query = search.trim().toLowerCase()
  const filtered = query
    ? fomods.filter(f => f.name.toLowerCase().includes(query))
    : fomods

  const selectedIndex = selectedName ? fomods.findIndex(f => f.name === selectedName) : -1
  const selectedEntry = selectedIndex >= 0 ? fomods[selectedIndex] : null
  const priority = selectedIndex >= 0 ? String(selectedIndex + 1).padStart(2, '0') : null

  const handleSelect = useCallback(
    (n: string) => {
      navigate(`/fomods/${encodeURIComponent(n)}`)
    },
    [navigate],
  )

  // Empty string keys the no-selection state; a real folder name is never empty,
  // so VfsTree/Inspector remount (resetting collapse + tab) on every change.
  const recordKey = selectedName ?? ''

  return (
    <div style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }}>
      {/* Header (46px) */}
      <div
        style={{
          height: 46,
          flexShrink: 0,
          display: 'flex',
          alignItems: 'center',
          gap: 12,
          padding: '0 18px',
          borderBottom: '1px solid var(--rule-soft)',
          boxShadow: 'var(--shadow-elevation-1)',
        }}
      >
        <Kicker num="02" label="Library" />
        <span aria-hidden="true" style={{ width: 1, height: 13, background: 'var(--rule-soft)' }} />
        <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--fs-micro)', color: 'var(--ink-4)', whiteSpace: 'nowrap' }}>
          {listLoading && fomods.length === 0
            ? '...'
            : `${filtered.length} ${filtered.length === 1 ? 'record' : 'records'}`}
        </span>

        <div style={{ position: 'relative', width: 230, marginLeft: 6 }}>
          <MIcon
            name="search"
            size={13}
            style={{
              position: 'absolute',
              left: 11,
              top: '50%',
              transform: 'translateY(-50%)',
              color: 'var(--ink-5)',
              pointerEvents: 'none',
            }}
          />
          <input
            type="text"
            placeholder="Filter mods..."
            value={search}
            onChange={e => { setSearch(e.target.value); }}
            aria-label="Filter mods"
            style={{
              width: '100%',
              padding: '7px 11px 7px 30px',
              border: '1px solid var(--rule)',
              borderRadius: 7,
              background: 'var(--sheet)',
              color: 'var(--ink)',
              fontFamily: 'var(--font-mono)',
              fontSize: 'var(--fs-label)',
              outline: 'none',
            }}
          />
        </div>

        <div style={{ flex: 1 }} />

        <button
          type="button"
          onClick={handleScanFomods}
          disabled={!pluginInstalled || scanRunning}
          title={pluginInstalled ? 'Rescan the MO2 mods for FOMOD selections' : 'Plugin not installed'}
          style={{
            display: 'inline-flex',
            alignItems: 'center',
            gap: 8,
            padding: '7px 15px',
            borderRadius: 7,
            background: 'var(--ink)',
            color: 'var(--sheet)',
            border: '1px solid var(--ink)',
            fontSize: 'var(--fs-body)',
            fontWeight: 600,
            fontFamily: 'inherit',
            cursor: !pluginInstalled || scanRunning ? 'not-allowed' : 'pointer',
            opacity: !pluginInstalled || scanRunning ? 0.55 : 1,
          }}
        >
          {!pluginInstalled ? (
            <MIcon name="link_off" size={13} />
          ) : (
            <MIcon name="radar" className={scanRunning ? 'm-spin' : undefined} size={13} />
          )}
          <span>{scanRunning ? 'Scanning' : 'Scan'}</span>
        </button>
      </div>

      {/* Body (triptych) */}
      <div style={{ flex: 1, minHeight: 0, display: 'flex' }}>
        <VfsTree key={`tree-${recordKey}`} outputTree={detail?.outputTree} hasSelection={selectedName !== null} />
        <RecordsList
          fomods={filtered}
          selectedName={selectedName}
          onSelect={handleSelect}
          loading={listLoading}
          totalCount={fomods.length}
        />
        <Inspector
          key={`inspector-${recordKey}`}
          name={selectedName}
          priority={priority}
          entry={selectedEntry}
          detail={detail}
          error={detailError}
          onRetry={retryDetail}
        />
      </div>
    </div>
  )
}
