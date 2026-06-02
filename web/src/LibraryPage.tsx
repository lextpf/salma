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

// Module 02 - Library. The selection-driven triptych, left to right: the
// confidence-banded records list (RecordsList), the VFS tree of the selected
// record (VfsTree), and the record inspector (Inspector). That is reading order
// as well as DOM order: choose a mod, see what it installs, then read why.
//
// The middle column is conditional. Below 950px of content width hideTree drops
// VfsTree entirely and the row becomes two columns; see useContentBreakpoints.
//
// Selection lives in the URL (/fomods/:name); the detail fetch is lifted here so
// the tree and inspector share one load. Search, scan, and tab state are local;
// VfsTree + Inspector remount on record change (via key) so their internal state
// (collapsed folders, active tab) resets cleanly.
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
      // No synchronous setListLoading(true) here. The initial value is already
      // true, and a background refresh (scan, retry) keeps the current list
      // visible instead of flashing a skeleton. The .catch path re-arms the
      // loading flag asynchronously while a retry is pending.
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

  // The chooser folds itself once a record is picked, and the spine unfolds it.
  // There is no explicit fold control: unfolding is something you do in order to
  // pick, and picking folds it again, so the cycle closes on its own.
  const [keepChooserOpen, setKeepChooserOpen] = useState(false)
  const chooserFolded = selectedName !== null && !keepChooserOpen
  const openChooser = useCallback(() => setKeepChooserOpen(true), [])

  // Picking a mod always folds the chooser, including when it was unfolded by
  // hand: choosing is the job the list is open for, so finishing it is the
  // moment the width should go back to the tree and the record.
  const handleSelect = useCallback(
    (n: string) => {
      setKeepChooserOpen(false)
      navigate(`/fomods/${encodeURIComponent(n)}`)
    },
    [navigate],
  )

  // Empty string keys the no-selection state; a real folder name is never empty,
  // so VfsTree/Inspector remount (resetting collapse + tab) on every change.
  const recordKey = selectedName ?? ''

  // The tree is the first thing to go when the window is docked narrow: it is
  // a preview of the selected record, not a way to reach one.
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
          onClick={handleScanFomods}
          disabled={!pluginInstalled || scanRunning}
          running={scanRunning}
          variant="primary"
          compact={compactToolbar}
          title={pluginInstalled ? 'Rescan the MO2 mods for FOMOD selections' : 'Plugin not installed'}
        />
      </ModuleHeader>

      {/* Body. Widths are never pinned: see useContentBreakpoints. The fill
          lives here and no column sets a plane or a fill step of its own, so
          the three read as one surface split by spacing. The chooser leads
          because it is the entry point, and folds to a spine once picked. */}
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
