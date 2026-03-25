import { useEffect, useRef, useState } from 'react'
import { Link } from 'react-router-dom'
import { listFomods, deleteFomod, isFetchUnavailableError } from '../api'
import FomodListItem from '../components/FomodListItem'
import type { FomodEntry } from '../types'

const RETRY_DELAY_MS = 2000

export default function FomodBrowserPage() {
  const [fomods, setFomods] = useState<FomodEntry[]>([])
  const [search, setSearch] = useState('')
  const [loading, setLoading] = useState(true)
  const [deletingName, setDeletingName] = useState<string | null>(null)
  const [deleteError, setDeleteError] = useState<string | null>(null)
  const retryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const inFlightRef = useRef(false)

  const clearRetryTimer = () => {
    if (retryTimerRef.current) {
      clearTimeout(retryTimerRef.current)
      retryTimerRef.current = null
    }
  }

  const load = (force = false) => {
    if (inFlightRef.current && !force) return
    setLoading(true)
    inFlightRef.current = true
    listFomods()
      .then(data => {
        setFomods(data)
        setLoading(false)
        clearRetryTimer()
      })
      .catch(e => {
        console.warn('[fomods] failed to load list, retrying', e)
        setLoading(true)
        if (!retryTimerRef.current) {
          retryTimerRef.current = setTimeout(() => {
            retryTimerRef.current = null
            load(true)
          }, RETRY_DELAY_MS)
        }
      })
      .finally(() => {
        inFlightRef.current = false
      })
  }

  useEffect(() => {
    load()
    return clearRetryTimer
  }, [])

  const handleDelete = async (name: string) => {
    setDeletingName(name)
    setDeleteError(null)
    try {
      await deleteFomod(name)
      setFomods(prev => prev.filter(f => f.name !== name))
    } catch (e) {
      if (isFetchUnavailableError(e)) {
        setDeleteError(`Failed to delete "${name}": backend unavailable`)
      } else {
        setDeleteError(`Failed to delete "${name}"`)
      }
      console.warn(`[fomods] failed to delete "${name}"`, e)
    } finally {
      setDeletingName(null)
    }
  }

  const filtered = fomods.filter(f =>
    f.name.toLowerCase().includes(search.toLowerCase())
  )

  return (
    <div className="fomods-page animate-fade-in h-[calc(100vh-4rem)] flex flex-col overflow-hidden">
      <header className="page-header page-header-fomods mb-6 flex items-end justify-between shrink-0">
        <div>
          <h1 className="page-title text-2xl font-bold text-on-surface flex items-center gap-3">
            <span className="inline-flex w-6 shrink-0 items-center justify-center">
              <i className="fa-duotone fa-solid fa-box-archive icon-gradient icon-gradient-spring text-2xl" />
            </span>
            FOMODs
          </h1>
          <p className="page-subtitle mt-1 pl-9 text-sm">Browse inferred FOMOD selection JSONs</p>
        </div>
        {!loading && (
          <span className="text-xs text-on-surface-variant tabular-nums">
            <span className="font-bold text-primary">{filtered.length}</span>
            <span className="mx-0.5">/</span>
            {fomods.length}
          </span>
        )}
      </header>

      {/* Search */}
      {loading ? (
        <div className="mb-5 shrink-0">
          <div className="skeleton-line h-[2.625rem] w-full rounded-xl" />
        </div>
      ) : (
        <div className="relative mb-5 shrink-0">
          <span className="absolute left-3.5 top-1/2 -translate-y-1/2 text-outline text-xs pointer-events-none">
            <i className="fa-duotone fa-solid fa-magnifying-glass" />
          </span>
          <input
            type="text"
            placeholder="Search mods..."
            value={search}
            onChange={e => setSearch(e.target.value)}
            className="w-full pl-10 pr-9 py-2.5 rounded-xl border border-outline-variant/40 bg-surface-container
                       text-sm text-on-surface placeholder:text-outline focus:outline-none focus:border-primary/40
                       focus:bg-surface-container-high focus:shadow-[0_0_20px_-8px_rgba(56,189,248,0.12)] transition-all"
          />
          {search && (
            <button
              onClick={() => setSearch('')}
              className="absolute right-3 top-1/2 -translate-y-1/2 text-outline hover:text-on-surface transition-colors"
            >
              <i className="fa-duotone fa-solid fa-circle-xmark text-sm" />
            </button>
          )}
        </div>
      )}

      {deleteError && (
        <div className="mb-3 shrink-0 rounded-xl border border-error/15 bg-error/5 px-4 py-2.5 text-xs text-error-light flex items-center gap-2">
          <i className="fa-duotone fa-solid fa-circle-exclamation duo-error" />
          <span className="flex-1">{deleteError}</span>
          <button onClick={() => setDeleteError(null)} className="text-error/60 hover:text-error transition-colors">
            <i className="fa-duotone fa-solid fa-xmark" />
          </button>
        </div>
      )}

      {loading ? (
        <div className="flex-1 min-h-0 py-2 px-2">
          <div className="flex flex-col gap-2.5">
            {[0, 1, 2].map(i => (
              <div key={i} className="rounded-xl border border-outline-variant/30 bg-surface-container/35 p-4">
                <div className="flex items-center gap-3">
                  <div className="w-9 h-9 rounded-lg skeleton-line" />
                  <div className="flex-1">
                    <div className="skeleton-line h-3.5 w-40 mb-2" />
                    <div className="skeleton-line h-3 w-28" />
                  </div>
                </div>
              </div>
            ))}
          </div>
        </div>
      ) : filtered.length === 0 ? (
        <div className="flex-1 min-h-0 py-10">
          <div className="empty-state-card">
            <div className="w-9 h-9 rounded-xl bg-surface-container-high/60 border border-outline-variant/25 flex items-center justify-center shadow-elevation-1">
              <i className="fa-duotone fa-solid fa-box-open icon-gradient icon-gradient-steel icon-sm" />
            </div>
            <div className="min-w-0">
              <p className="text-sm text-on-surface">
                {fomods.length === 0 ? 'No FOMOD JSONs found.' : 'No results match your search.'}
              </p>
              {fomods.length === 0 && (
                <p className="ui-micro mt-1">
                  Configure MO2 path in <Link to="/settings" className="text-primary hover:underline underline-offset-2">Settings</Link>.
                </p>
              )}
            </div>
          </div>
        </div>
      ) : (
        <div className="flex-1 min-h-0 overflow-y-auto scroll-pane px-3">
          <div className="flex flex-col gap-2 pb-3">
            {filtered.map(f => (
              <FomodListItem key={f.name} fomod={f} onDelete={handleDelete} disabled={deletingName !== null} />
            ))}
          </div>
        </div>
      )}
    </div>
  )
}
