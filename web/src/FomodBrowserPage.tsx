import { useEffect, useRef, useState } from 'react'
import { Link } from 'react-router-dom'
import { listFomods, deleteFomod, isFetchUnavailableError } from './api'
import FomodListItem from './components/FomodListItem'
import Section from './components/Section'
import type { FomodEntry } from './types'

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

  const isEmpty = !loading && filtered.length === 0

  return (
    <div className="page-fill">
      {/* Header */}
      <header style={{ marginBottom: 28, flexShrink: 0 }}>
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
            02.
          </span>
          <span>Sec. Parsed FOMOD library</span>
        </p>

        <div className="flex items-end reveal reveal-delay-2" style={{ gap: 20, flexWrap: 'wrap' }}>
          <h1
            className="display-serif"
            style={{ fontSize: 92, lineHeight: 0.92, color: 'var(--ink)', margin: 0 }}
          >
            FOMODs<span style={{ color: 'var(--accent)' }}>.</span>
          </h1>
          {!loading && (
            <span style={{ marginBottom: 16 }}>
              <span
                className="display-serif tabular-nums"
                style={{ fontSize: 26, color: 'var(--ink)', letterSpacing: '-0.02em' }}
              >
                {filtered.length}
              </span>
              <span
                style={{
                  fontFamily: 'var(--font-mono)',
                  fontSize: 11,
                  color: 'var(--ink-4)',
                  marginLeft: 6,
                  letterSpacing: '0.04em',
                }}
              >
                / {fomods.length} entries
              </span>
            </span>
          )}
        </div>

        <p
          className="reveal reveal-delay-3 display-serif-italic"
          style={{
            margin: '18px 0 0',
            fontSize: 15,
            color: 'var(--ink-3)',
            maxWidth: 620,
            lineHeight: 1.55,
          }}
        >
          Browse the inferred FOMOD selection JSONs that salma has parsed from your installed mods. Click an entry to inspect its installation steps.
        </p>
      </header>

      {deleteError && (
        <div
          className="reveal"
          style={{
            marginBottom: 16,
            padding: '10px 14px',
            background: 'var(--paper-3)',
            border: '1px solid var(--rule)',
            borderRadius: 'var(--radius-sm)',
            display: 'flex',
            alignItems: 'center',
            gap: 10,
            flexShrink: 0,
          }}
        >
          <span className="dot-status dot-status-error" />
          <span style={{ flex: 1, fontSize: 12, color: 'var(--danger)' }}>{deleteError}</span>
          <button
            onClick={() => setDeleteError(null)}
            type="button"
            style={{
              border: 'none',
              background: 'transparent',
              color: 'var(--ink-4)',
              cursor: 'pointer',
              padding: 4,
            }}
          >
            <i className="fa-duotone fa-solid fa-xmark" />
          </button>
        </div>
      )}

      <Section
          n="01"
          label="Library"
          title="Browse FOMODs"
          corner="01"
          bodyPadding="none"
          className={`reveal reveal-delay-4${(isEmpty || loading) ? '' : ' atelier-section-fill'}`}
          meta={
            <div style={{ position: 'relative', minWidth: 240 }}>
              <i
                className="fa-duotone fa-solid fa-magnifying-glass"
                style={{
                  position: 'absolute',
                  left: 10,
                  top: '50%',
                  transform: 'translateY(-50%)',
                  fontSize: 12,
                  color: 'var(--ink-4)',
                  pointerEvents: 'none',
                }}
              />
              <input
                type="text"
                placeholder="Search mods..."
                value={search}
                onChange={e => setSearch(e.target.value)}
                style={{
                  width: '100%',
                  padding: '6px 28px 6px 28px',
                  border: '1px solid var(--rule)',
                  borderRadius: 'var(--radius-sm)',
                  background: 'var(--card)',
                  fontFamily: 'var(--font-body)',
                  fontSize: 12.5,
                  color: 'var(--ink)',
                  outline: 'none',
                  letterSpacing: '-0.005em',
                }}
              />
              {search && (
                <button
                  type="button"
                  onClick={() => setSearch('')}
                  style={{
                    position: 'absolute',
                    right: 8,
                    top: '50%',
                    transform: 'translateY(-50%)',
                    border: 'none',
                    background: 'transparent',
                    color: 'var(--ink-4)',
                    cursor: 'pointer',
                    padding: 2,
                  }}
                >
                  <i className="fa-duotone fa-solid fa-circle-xmark" style={{ fontSize: 12 }} />
                </button>
              )}
            </div>
          }
        >
          <div
            className="scroll-pane"
            style={loading ? { overflow: 'visible' } : isEmpty ? { overflow: 'auto' } : { flex: 1, minHeight: 0, overflow: 'auto' }}
          >
            {loading ? (
              <div>
                {[0, 1, 2, 3].map(i => (
                  <div
                    key={i}
                    className="grid"
                    style={{
                      gridTemplateColumns: '32px 1fr 110px 90px 100px 28px 24px',
                      gap: 16,
                      padding: '14px 28px',
                      borderBottom: i === 3 ? 'none' : '1px solid var(--rule-soft)',
                      alignItems: 'center',
                    }}
                  >
                    <div className="skeleton-line" style={{ width: 28, height: 28, borderRadius: 'var(--radius-sm)' }} />
                    <div className="skeleton-line" style={{ height: 14, width: ['78%', '62%', '85%', '54%'][i] }} />
                    <div className="skeleton-line" style={{ width: 80, height: 10 }} />
                    <div className="skeleton-line" style={{ width: 60, height: 10 }} />
                    <div className="skeleton-line" style={{ width: 80, height: 10 }} />
                    <div />
                    <div />
                  </div>
                ))}
              </div>
            ) : filtered.length === 0 ? (
              <div style={{ padding: 28 }}>
                <div className="empty-state-card">
                  <div
                    style={{
                      width: 36,
                      height: 36,
                      borderRadius: '50%',
                      border: '1px solid var(--rule-strong)',
                      background: 'var(--paper-3)',
                      display: 'flex',
                      alignItems: 'center',
                      justifyContent: 'center',
                      color: 'var(--ink-3)',
                      flexShrink: 0,
                    }}
                  >
                    <i className="fa-duotone fa-solid fa-folder-open" style={{ fontSize: 14 }} />
                  </div>
                  <div style={{ flex: 1 }}>
                    <p
                      className="display-serif-italic"
                      style={{ fontSize: 18, color: 'var(--ink)', lineHeight: 1.2 }}
                    >
                      {fomods.length === 0 ? 'No FOMOD JSONs found' : 'No matches'}
                      <span className="display-period">.</span>
                    </p>
                    <p className="timestamp-print" style={{ marginTop: 4 }}>
                      {fomods.length === 0 ? (
                        <>
                          // configure mo2 path in{' '}
                          <Link to="/settings" style={{ color: 'var(--accent)', textDecoration: 'underline' }}>
                            settings
                          </Link>
                          , then run a scan
                        </>
                      ) : (
                        <>// no entries match "{search}"</>
                      )}
                    </p>
                  </div>
                </div>
              </div>
            ) : (
              <div>
                {filtered.map(f => (
                  <FomodListItem
                    key={f.name}
                    fomod={f}
                    onDelete={handleDelete}
                    disabled={deletingName !== null}
                  />
                ))}
              </div>
            )}
          </div>
        </Section>
    </div>
  )
}
