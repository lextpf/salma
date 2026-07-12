import { useEffect, useRef, useState } from 'react'
import { getConfig, isFetchUnavailableError, putConfig } from './api'
import Kicker from './comps/Kicker'
import MIcon from './comps/MIcon'
import ConfigSheet from './comps/settings/ConfigSheet'
import type { AppConfig } from './types'

const MONO = 'var(--font-mono)'
const RETRY_DELAY_MS = 2000

// Syntactic validity: a non-empty path with no characters Windows forbids in a
// directory. Gates the Save button and drives the sheet's header validity dot.
function isPathValid(p: string): boolean {
  return p.trim().length > 0 && !/[*?<>|]/.test(p)
}

export default function SettingsPage() {
  const [config, setConfig] = useState<AppConfig | null>(null)
  const [modsPath, setModsPath] = useState('')
  const [saving, setSaving] = useState(false)
  const [message, setMessage] = useState<{ type: 'success' | 'error'; text: string } | null>(null)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [testArgs, setTestArgs] = useState(() => localStorage.getItem('salma_test_args') || '')
  const retryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const retryCountRef = useRef(0)
  const pathValid = isPathValid(modsPath)

  const clearRetryTimer = () => {
    if (retryTimerRef.current) {
      clearTimeout(retryTimerRef.current)
      retryTimerRef.current = null
    }
  }

  const loadConfig = () => {
    setLoadError(null)
    getConfig()
      .then((c) => {
        setConfig(c)
        setModsPath(c.mo2ModsPath)
        retryCountRef.current = 0
        clearRetryTimer()
      })
      .catch((e) => {
        retryCountRef.current++
        console.warn(`[settings] failed to load config (attempt ${retryCountRef.current})`, e)
        clearRetryTimer()
        if (retryCountRef.current >= 3) {
          setLoadError('Unable to connect to backend. Please check the server is running.')
        } else {
          retryTimerRef.current = setTimeout(loadConfig, RETRY_DELAY_MS)
        }
      })
  }

  useEffect(() => {
    loadConfig()
    return clearRetryTimer
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const handleSave = async () => {
    setSaving(true)
    setMessage(null)
    try {
      const updated = await putConfig({ mo2ModsPath: modsPath })
      setConfig(updated)
      localStorage.setItem('salma_test_args', testArgs)
      setMessage({ type: 'success', text: 'Configuration saved successfully.' })
    } catch (e) {
      if (isFetchUnavailableError(e)) {
        console.warn('[settings] save request failed due to unavailable backend', e)
        setMessage({ type: 'error', text: 'Backend unavailable. Please try again in a moment.' })
      } else {
        setMessage({ type: 'error', text: e instanceof Error ? e.message : 'Failed to save' })
      }
    } finally {
      setSaving(false)
    }
  }

  return (
    <div style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }}>
      {/* 46px section header */}
      <div
        style={{
          height: 46,
          flexShrink: 0,
          display: 'flex',
          alignItems: 'center',
          gap: 12,
          padding: '0 18px',
          borderBottom: '1px solid var(--rule-soft)',
        }}
      >
        <Kicker num="04" label="Configuration" />
      </div>

      {/* Scrolling body */}
      <div
        style={{
          flex: 1,
          minHeight: 0,
          overflowY: 'auto',
          overflowX: 'hidden',
          padding: 18,
        }}
      >
        <div style={{ width: '100%', maxWidth: 960 }}>
          {loadError ? (
            <div
              style={{
                border: '1px solid var(--rule)',
                borderRadius: 11,
                padding: '24px 24px',
                background: 'var(--sheet)',
                boxShadow: 'var(--shadow-elevation-1)',
              }}
            >
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 10 }}>
                <span
                  style={{
                    width: 6,
                    height: 6,
                    borderRadius: '50%',
                    background: 'var(--danger)',
                  }}
                />
                <span
                  style={{
                    fontFamily: MONO,
                    fontSize: 'var(--fs-micro)',
                    letterSpacing: '0.14em',
                    textTransform: 'uppercase',
                    color: 'var(--danger)',
                  }}
                >
                  Connection failed
                </span>
              </div>
              <p
                style={{
                  margin: '0 0 14px',
                  fontSize: 'var(--fs-title)',
                  lineHeight: 'var(--lh-body)',
                  color: 'var(--ink-2)',
                }}
              >
                {loadError}
              </p>
              <button
                type="button"
                className="tool-btn"
                onClick={() => {
                  retryCountRef.current = 0
                  loadConfig()
                }}
              >
                <MIcon name="sync" size={13} />
                <span>Retry</span>
              </button>
            </div>
          ) : !config ? (
            <div
              style={{
                border: '1px solid var(--rule)',
                borderRadius: 11,
                overflow: 'hidden',
                background: 'var(--sheet)',
                boxShadow: 'var(--shadow-elevation-2)',
              }}
            >
              <div
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'space-between',
                  padding: '10px 16px',
                  background: 'var(--card)',
                  borderBottom: '1px solid var(--rule-soft)',
                }}
              >
                <span style={{ fontFamily: MONO, fontSize: 'var(--fs-label)', color: 'var(--ink-4)' }}>
                  salma.json
                </span>
                <div
                  className="skeleton-line"
                  style={{ height: 22, width: 64, borderRadius: 7 }}
                />
              </div>
              <div style={{ padding: '15px 18px' }}>
                {[58, 28, 50, 66, 18, 28, 56].map((w, i) => (
                  <div
                    key={i}
                    className="skeleton-line"
                    style={{ height: 11, width: `${w}%`, margin: '8px 0' }}
                  />
                ))}
              </div>
            </div>
          ) : (
            <ConfigSheet
              config={config}
              modsPath={modsPath}
              onModsPathChange={setModsPath}
              testArgs={testArgs}
              onTestArgsChange={setTestArgs}
              valid={pathValid}
              saving={saving}
              onSave={handleSave}
              saveMessage={message}
            />
          )}
        </div>
      </div>
    </div>
  )
}
