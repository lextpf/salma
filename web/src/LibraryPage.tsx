import { useCallback, useEffect, useRef, useState } from 'react'

import { useNavigate, useParams } from 'react-router-dom'
import { listFomods } from './api'
import { useScanJob } from './useScanJob'
import { useSystemStatus } from './useSystemStatus'
import { useRecordDetail } from './useRecordDetail'
import { useContentBreakpoints } from './useViewportNarrow'
import Button from './comps/Button'
import MIcon from './comps/MIcon'
import ModuleHeader from './comps/ModuleHeader'
import { VRule } from './comps/Rule'
import VfsTree from './comps/VfsTree'
import RecordsList from './comps/RecordsList'
import Inspector from './comps/Inspector'
import type { FomodEntry } from './types'

const RETRY_DELAY_MS = 2000

export default function LibraryPage() {
  // URL selection owns the shared detail request. keyed views reset record-local state.
  const { name } = useParams<{ name: string }>()
  const navigate = useNavigate()
  const selectedName = name ? decodeURIComponent(name) : null

  const [fomods, setFomods] = useState<FomodEntry[]>([])
  const [listLoading, setListLoading] = useState(true)
  const [search, setSearch] = useState('')
  const retryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const inFlightRef = useRef(false)
  // initialize the latest callback before retry setup.
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
      // keep the current list visible during background refresh.
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

  // let retry timers use the current loader without resubscribing.
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

  const [keepChooserOpen, setKeepChooserOpen] = useState(false)
  const chooserFolded = selectedName !== null && !keepChooserOpen
  const openChooser = useCallback(() => { setKeepChooserOpen(true); }, [])

  const handleSelect = useCallback(
    (n: string) => {
      setKeepChooserOpen(false)
      void navigate(`/fomods/${encodeURIComponent(n)}`)
    },
    [navigate],
  )

  const recordKey = selectedName ?? ''

  const { hideTree, chipDial, compactToolbar } = useContentBreakpoints()

  return (
    <div style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }}>
      <ModuleHeader num="02" label="Library">
        <VRule height={18} />
        <span
          className="tabular-nums"
          style={{
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-label)',
            color: 'var(--ink-5)',
            whiteSpace: 'nowrap',
            flexShrink: 0,
          }}
        >
          {listLoading && fomods.length === 0
            ? '...'
            : `${filtered.length} ${filtered.length === 1 ? 'record' : 'records'}`}
        </span>

        <div style={{ position: 'relative', flex: '0 1 250px', minWidth: 126, marginLeft: 6 }}>
          <MIcon
            name="search"
            size={15}
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
              height: 30,
              padding: '0 11px 0 32px',
              border: '1px solid var(--rule-ctrl)',
              borderRadius: 'var(--radius-input)',
              background: 'var(--input)',
              color: 'var(--ink)',
              fontFamily: 'var(--font-mono)',
              fontSize: 'var(--fs-mono)',
              outline: 'none',
            }}
          />
        </div>

        <div style={{ flex: 1 }} />

        <Button
          icon={pluginInstalled ? 'radar' : 'link_off'}
          label={scanRunning ? 'Scanning' : 'Scan'}
          onClick={() => { void handleScanFomods(); }}
          disabled={!pluginInstalled || scanRunning}
          running={scanRunning}
          variant="primary"
          compact={compactToolbar}
          title={pluginInstalled ? 'Rescan the MO2 mods for FOMOD selections' : 'Plugin not installed'}
        />
      </ModuleHeader>

      <div style={{ flex: 1, minHeight: 0, display: 'flex', gap: 22, background: 'var(--paper)' }}>
        <RecordsList
          fomods={filtered}
          selectedName={selectedName}
          onSelect={handleSelect}
          loading={listLoading}
          totalCount={fomods.length}
          tight={chipDial}
          collapsed={chooserFolded}
          onExpand={openChooser}
        />
        {!hideTree && (
          <VfsTree
            key={`tree-${recordKey}`}
            outputTree={detail?.outputTree}
            hasSelection={selectedName !== null}
            repro={detail?.diagnostics?.repro}
            reproDetail={detail?.reproDetail}
          />
        )}
        <Inspector
          key={`inspector-${recordKey}`}
          name={selectedName}
          priority={priority}
          entry={selectedEntry}
          detail={detail}
          error={detailError}
          onRetry={retryDetail}
          tight={chipDial}
        />
      </div>
    </div>
  )
}
