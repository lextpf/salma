import { useEffect, useRef, useState } from 'react'
import { getConfig, isFetchUnavailableError, putConfig } from './api'
import Button from './comps/Button'
import ModuleHeader from './comps/ModuleHeader'
import { VRule } from './comps/Rule'
import ConfigSheet from './comps/ConfigSheet'
import { getTailLogs, getTestArgs, setTailLogs, setTestArgs } from './prefs'
import { useContentBreakpoints } from './useViewportNarrow'
import type { AppConfig } from './types'

const MONO = 'var(--font-mono)'
const RETRY_DELAY_MS = 2000

// this is a syntax screen only. the server checks whether the path exists.
function isPathValid(p: string): boolean {
  return p.trim().length > 0 && !/[*?<>|]/.test(p)
}

function pad2(n: number): string {
  return String(n).padStart(2, '0')
}

export default function SettingsPage() {
  const { compactToolbar } = useContentBreakpoints()
  const [config, setConfig] = useState<AppConfig | null>(null)
  const [modsPath, setModsPath] = useState('')
  const [saving, setSaving] = useState(false)
  const [message, setMessage] = useState<{ type: 'success' | 'error'; text: string } | null>(null)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [testArgs, setTestArgsState] = useState(getTestArgs)
  const [savedTestArgs, setSavedTestArgs] = useState(getTestArgs)
  const [tailLogs, setTailLogsState] = useState(getTailLogs)
  const [savedTailLogs, setSavedTailLogs] = useState(getTailLogs)
  const [savedAt, setSavedAt] = useState<Date | null>(null)
  const retryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const retryCountRef = useRef(0)

  const pathValid = isPathValid(modsPath)
  const dirty =
    config != null &&
    (modsPath !== config.mo2ModsPath || testArgs !== savedTestArgs || tailLogs !== savedTailLogs)

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

  const handleRevert = () => {
    if (!config) return
    setModsPath(config.mo2ModsPath)
    setTestArgsState(savedTestArgs)
    setTailLogsState(savedTailLogs)
    setMessage(null)
  }

  const handleSave = async () => {
    setSaving(true)
    setMessage(null)
    try {
      const updated = await putConfig({ mo2ModsPath: modsPath })
      setConfig(updated)
      setModsPath(updated.mo2ModsPath)
      setTestArgs(testArgs)
      setSavedTestArgs(testArgs)
      setTailLogs(tailLogs)
      setSavedTailLogs(tailLogs)
      setSavedAt(new Date())
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

  const savedLabel = savedAt
    ? `saved ${pad2(savedAt.getHours())}:${pad2(savedAt.getMinutes())}`
    : dirty ? 'unsaved changes' : 'no changes'

  return (
    <div style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }}>
      <ModuleHeader num="04" label="Settings">
        <VRule height={18} />
        <span
          style={{
            fontFamily: MONO,
            fontSize: 'var(--fs-label)',
            color: dirty ? 'var(--brass)' : 'var(--ink-5)',
            whiteSpace: 'nowrap',
          }}
        >
          salma.json &middot; {savedLabel}
        </span>

        <div style={{ flex: 1 }} />

        <Button
          icon="restart_alt"
          label="Revert"
          onClick={handleRevert}
          disabled={!dirty || saving}
          compact={compactToolbar}
        />
        <Button
          icon="check"
          label={saving ? 'Applying...' : 'Apply'}
          variant="primary"
          onClick={() => { void handleSave(); }}
          disabled={!dirty || !pathValid || saving}
          running={saving}
          compact={compactToolbar}
        />
      </ModuleHeader>

      <div
        style={{
          flex: 1,
          minHeight: 0,
          overflowY: 'auto',
          overflowX: 'hidden',
          padding: '24px 28px 32px',
        }}
      >
        <div style={{ width: '100%', maxWidth: 840, background: 'var(--paper)' }}>
          {loadError ? (
            <div
              style={{
                padding: '16px 14px',
                background: 'var(--danger-wash)',
              }}
            >
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 10 }}>
                <span
                  aria-hidden="true"
                  style={{ width: 6, height: 6, borderRadius: 'var(--radius-full)', background: 'var(--danger)' }}
                />
                <span
                  style={{
                    fontFamily: MONO,
                    fontSize: 'var(--fs-micro)',
                    fontWeight: 600,
                    letterSpacing: 'var(--tr-kicker)',
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
                  textWrap: 'pretty',
                }}
              >
                {loadError}
              </p>
              <Button
                icon="sync"
                label="Retry"
                onClick={() => {
                  retryCountRef.current = 0
                  loadConfig()
                }}
              />
            </div>
          ) : !config ? (
            <div
              style={{
                padding: '15px 14px',
              }}
            >
              {[58, 28, 50, 66, 18, 28, 56].map((w, i) => (
                <div
                  key={i}
                  className="skeleton-line"
                  style={{ height: 11, width: `${w}%`, margin: '10px 0' }}
                />
              ))}
            </div>
          ) : (
            <ConfigSheet
              config={config}
              modsPath={modsPath}
              onModsPathChange={setModsPath}
              testArgs={testArgs}
              onTestArgsChange={setTestArgsState}
              tailLogs={tailLogs}
              onTailLogsChange={setTailLogsState}
              valid={pathValid}
              saveMessage={message}
            />
          )}
        </div>
      </div>
    </div>
  )
}
