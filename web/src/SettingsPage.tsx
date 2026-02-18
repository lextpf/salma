import { useEffect, useRef, useState } from 'react'
import { getConfig, isFetchUnavailableError, putConfig } from './api'
import Section from './components/Section'
import type { AppConfig } from './types'

const RETRY_DELAY_MS = 2000

const inputStyle: React.CSSProperties = {
  width: '100%',
  padding: '10px 14px',
  border: '1px solid var(--rule)',
  borderRadius: 'var(--radius-sm)',
  background: 'var(--paper-2)',
  fontFamily: 'var(--font-mono)',
  fontSize: 13,
  color: 'var(--ink)',
  outline: 'none',
  letterSpacing: 0,
  transition: 'border-color 150ms ease, background-color 150ms ease',
}

const inputReadonlyStyle: React.CSSProperties = {
  ...inputStyle,
  background: 'var(--paper-3)',
  color: 'var(--ink-3)',
  cursor: 'not-allowed',
}

function FieldRow({
  label,
  hint,
  children,
}: {
  label: string
  hint?: React.ReactNode
  children: React.ReactNode
}) {
  return (
    <div
      className="grid"
      style={{
        gridTemplateColumns: '220px 1fr',
        gap: 24,
        padding: '20px 28px',
        borderBottom: '1px solid var(--rule-soft)',
        alignItems: 'flex-start',
      }}
    >
      <div>
        <p className="ui-label" style={{ marginBottom: 6 }}>{label}</p>
        {hint && (
          <p className="timestamp-print" style={{ fontSize: 11, lineHeight: 1.5 }}>{hint}</p>
        )}
      </div>
      <div style={{ minWidth: 0 }}>{children}</div>
    </div>
  )
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
  const isPathValid = (p: string) => p.trim().length > 0 && !/[*?<>|]/.test(p)
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
      .then(c => {
        setConfig(c)
        setModsPath(c.mo2ModsPath)
        retryCountRef.current = 0
        clearRetryTimer()
      })
      .catch(e => {
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
    <div className="page-fill">
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
            04.
          </span>
          <span>Sec. Configuration</span>
        </p>

        <h1
          className="reveal reveal-delay-2 display-serif"
          style={{ fontSize: 92, lineHeight: 0.92, color: 'var(--ink)', margin: 0 }}
        >
          Settings<span style={{ color: 'var(--accent)' }}>.</span>
        </h1>

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
          Configure MO2 integration paths and test runner arguments. Changes persist across sessions.
        </p>
      </header>

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
              Connection failed
            </span>
          </div>
          <p
            className="display-serif-italic"
            style={{ fontSize: 16, color: 'var(--ink)', marginBottom: 12 }}
          >
            {loadError}
          </p>
          <button
            type="button"
            className="tool-btn"
            onClick={() => { retryCountRef.current = 0; loadConfig() }}
          >
            <i className="fa-duotone fa-solid fa-arrows-spin" style={{ fontSize: 12 }} />
            <span>Retry</span>
          </button>
        </div>
      ) : !config ? (
        <div className="reveal reveal-delay-4" style={{ marginBottom: 28 }}>
          <Section n="01" label="Paths" title="MO2 instance" corner="01" bodyPadding="none">
            <div
              className="grid"
              style={{
                gridTemplateColumns: '220px 1fr',
                gap: 24,
                padding: '20px 28px',
                borderBottom: '1px solid var(--rule-soft)',
                alignItems: 'flex-start',
              }}
            >
              <div>
                <div className="skeleton-line" style={{ height: 10, width: 100, marginBottom: 8 }} />
                <div className="skeleton-line" style={{ height: 11, width: 180 }} />
              </div>
              <div className="skeleton-line" style={{ height: 36, width: '100%' }} />
            </div>
          </Section>
        </div>
      ) : (
        <>
          {/* Section 01 - Paths */}
          <div className="reveal reveal-delay-4" style={{ marginBottom: 16, flexShrink: 0 }}>
            <Section n="01" label="Paths" title="MO2 instance" corner="01" bodyPadding="none">
              <FieldRow
                label="Mods path"
                hint={<>// e.g. <span style={{ color: 'var(--ink-2)' }}>D:\MO2\mods</span></>}
              >
                <input
                  type="text"
                  value={modsPath}
                  onChange={e => setModsPath(e.target.value)}
                  placeholder="D:\MO2\mods"
                  style={inputStyle}
                  onFocus={e => { e.currentTarget.style.borderColor = 'var(--accent)' }}
                  onBlur={e => { e.currentTarget.style.borderColor = 'var(--rule)' }}
                />
                {config.mo2ModsPath && (
                  <p
                    className="flex items-center timestamp-print"
                    style={{
                      gap: 6,
                      marginTop: 8,
                      color: config.mo2ModsPathValid ? 'var(--moss)' : 'var(--danger)',
                    }}
                  >
                    <span
                      className={`dot-status ${config.mo2ModsPathValid ? 'dot-status-on' : 'dot-status-error'}`}
                    />
                    {config.mo2ModsPathValid
                      ? 'path exists and is a directory'
                      : 'path does not exist or is not a directory'}
                  </p>
                )}
              </FieldRow>

              {config.fomodOutputDir && (
                <FieldRow
                  label="FOMOD output"
                  hint={<>// auto-generated from the mods path - read-only</>}
                >
                  <input
                    type="text"
                    value={config.fomodOutputDir}
                    readOnly
                    aria-readonly="true"
                    style={inputReadonlyStyle}
                  />
                </FieldRow>
              )}
            </Section>
          </div>

          {/* Section 02 - Test runner */}
          <div className="reveal reveal-delay-5" style={{ flexShrink: 0 }}>
            <Section n="02" label="Test" title="Test runner" corner="02" bodyPadding="none">
              <FieldRow
                label="Script arguments"
                hint={
                  <>
                    // passed to <span style={{ color: 'var(--ink-2)' }}>test_all.py</span>{' '}
                    when running tests
                  </>
                }
              >
                <input
                  type="text"
                  value={testArgs}
                  onChange={e => setTestArgs(e.target.value)}
                  placeholder='--separator "My Separator" --limit 10'
                  style={inputStyle}
                  onFocus={e => { e.currentTarget.style.borderColor = 'var(--accent)' }}
                  onBlur={e => { e.currentTarget.style.borderColor = 'var(--rule)' }}
                />
              </FieldRow>

              <div className="atelier-section-toolbar">
                {message && (
                  <span
                    className="flex items-center"
                    style={{
                      gap: 6,
                      fontFamily: 'var(--font-mono)',
                      fontSize: 11,
                      color: message.type === 'success' ? 'var(--moss)' : 'var(--danger)',
                    }}
                  >
                    <span
                      className={`dot-status ${message.type === 'success' ? 'dot-status-on' : 'dot-status-error'}`}
                    />
                    {message.text}
                  </span>
                )}
                <div style={{ flex: 1 }} />
                <button
                  type="button"
                  className="tool-btn tool-btn-primary"
                  onClick={handleSave}
                  disabled={saving || !pathValid}
                >
                  <i
                    className={`fa-duotone fa-solid fa-floppy-disk${saving ? ' fa-beat' : ''}`}
                    style={{ fontSize: 13 }}
                  />
                  <span>{saving ? 'Saving...' : 'Save configuration'}</span>
                </button>
              </div>
            </Section>
          </div>
        </>
      )}
    </div>
  )
}
