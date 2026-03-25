import { useEffect, useRef, useState } from 'react'
import { getConfig, isFetchUnavailableError, putConfig } from '../api'
import type { AppConfig } from '../types'

const RETRY_DELAY_MS = 2000

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
  const pathFieldClass = "w-full px-3.5 py-2.5 rounded-xl border border-outline-variant/60 bg-surface-container-high text-sm text-on-surface placeholder:text-outline focus:outline-none focus:border-primary/50 focus:bg-surface-container-highest transition-all log-viewer"
  const readonlyPathFieldClass = "w-full px-3.5 pr-28 py-2.5 rounded-xl border border-outline-variant/40 bg-surface-dim/55 text-sm text-on-surface-variant focus:outline-none cursor-not-allowed transition-all log-viewer"

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
    <div className="animate-fade-in">
      <header className="page-header page-header-settings mb-6">
        <h1 className="page-title text-2xl font-bold text-on-surface flex items-center gap-3">
          <span className="inline-flex w-6 shrink-0 items-center justify-center">
            <i className="fa-duotone fa-solid fa-gear icon-gradient icon-gradient-steel text-2xl" />
          </span>
          Settings
        </h1>
        <p className="page-subtitle mt-1 pl-9 text-sm">Configure MO2 integration paths</p>
      </header>

      {!config ? (
        loadError ? (
          <div className="max-w-2xl rounded-2xl border border-error/15 bg-error/5 p-6 flex flex-col items-center gap-4">
            <i className="fa-duotone fa-solid fa-circle-exclamation duo-error text-2xl" />
            <p className="text-sm text-error-light text-center">{loadError}</p>
            <button
              onClick={() => { retryCountRef.current = 0; loadConfig() }}
              className="rounded-xl px-4 py-2 text-xs font-semibold bg-error/12 text-error-light border border-error/20 hover:bg-error/24 transition-colors"
            >
              <i className="fa-duotone fa-solid fa-rotate-right mr-1.5" />Retry
            </button>
          </div>
        ) : (
          <div className="max-w-2xl space-y-3 rounded-2xl">
            <div className="rounded-xl border border-outline-variant/30 bg-surface-container/35 p-4">
              <div className="skeleton-line h-3.5 w-32 mb-3" />
              <div className="skeleton-line h-9 w-full mb-2.5" />
              <div className="skeleton-line h-3 w-40" />
            </div>
            <div className="rounded-xl border border-outline-variant/30 bg-surface-container/35 p-4">
              <div className="skeleton-line h-3.5 w-44 mb-3" />
              <div className="skeleton-line h-9 w-full" />
            </div>
          </div>
        )
      ) : (
      <div className="max-w-2xl space-y-5">
        {/* Mods Path */}
        <div className="rounded-2xl border border-outline-variant/30 bg-surface-container/35 p-5">
          <label className="flex items-center gap-2 ui-xs font-semibold text-on-surface mb-3 uppercase tracking-wider">
            <i className="fa-duotone fa-solid fa-folder-tree icon-gradient icon-gradient-steel icon-sm" />
            MO2 Mods Path
          </label>
          <input
            type="text"
            value={modsPath}
            onChange={e => setModsPath(e.target.value)}
            placeholder="e.g. D:\MO2\mods"
            className={pathFieldClass}
          />
          {config.mo2ModsPath && (
            <p className={`flex items-center gap-1.5 text-[0.675rem] mt-2.5 ${config.mo2ModsPathValid ? 'settings-valid-text' : 'text-error'}`}>
              <i className={`fa-duotone fa-solid ${config.mo2ModsPathValid ? 'fa-circle-check settings-valid-icon' : 'fa-circle-xmark duo-error'} text-[0.6rem]`} />
              {config.mo2ModsPathValid ? 'Path exists and is a directory' : 'Path does not exist or is not a directory'}
            </p>
          )}
        </div>

        {/* Derived info */}
        {config.fomodOutputDir && (
          <div className="rounded-2xl border border-outline-variant/30 bg-surface-container/35 p-5">
            <p className="flex items-center gap-2 ui-xs font-semibold text-on-surface mb-2.5 uppercase tracking-wider">
              <i className="fa-duotone fa-solid fa-folder-open icon-gradient icon-gradient-steel icon-sm" />
              FOMOD Output Directory
            </p>
            <div className="relative">
              <input
                type="text"
                value={config.fomodOutputDir}
                readOnly
                aria-readonly="true"
                title="This field is generated automatically and cannot be edited."
                className={readonlyPathFieldClass}
              />
              <span className="pointer-events-none absolute right-2.5 top-1/2 -translate-y-1/2 inline-flex items-center gap-1 rounded-md border border-outline-variant/40 bg-surface-container px-2 py-0.5 text-[0.62rem] font-semibold uppercase tracking-wide text-on-surface-variant">
                <i className="fa-duotone fa-solid fa-lock text-[0.58rem]" />
                Read-only
              </span>
            </div>
            <p className="mt-2 text-[0.66rem] text-on-surface-variant/85">
              Auto-generated from the MO2 Mods Path.
            </p>
          </div>
        )}

        {/* Test Script Arguments */}
        <div className="rounded-2xl border border-outline-variant/30 bg-surface-container/35 p-5">
          <label className="flex items-center gap-2 ui-xs font-semibold text-on-surface mb-1.5 uppercase tracking-wider">
            <i className="fa-duotone fa-solid fa-flask icon-gradient icon-gradient-steel icon-sm" />
            Test Script Arguments
          </label>
          <p className="text-[0.66rem] text-on-surface-variant/85 mb-3 pl-0.5">
            Arguments passed to <code className="px-1 py-0.5 rounded bg-surface-container-high text-[0.62rem]">test.py</code> when running tests.
            e.g. <code className="px-1 py-0.5 rounded bg-surface-container-high text-[0.62rem]">--separator "My Separator" --limit 5 --no-full</code>
          </p>
          <input
            type="text"
            value={testArgs}
            onChange={e => setTestArgs(e.target.value)}
            placeholder='e.g. --separator "My Separator" --limit 10'
            className={pathFieldClass}
          />
        </div>

        {/* Messages */}
        {message && (
          <div className={`rounded-xl border px-4 py-3 text-xs flex items-center gap-2 ${
            message.type === 'success'
              ? 'border-success/15 bg-success/5 text-success-light'
              : 'border-error/15 bg-error/5 text-error-light'
          }`}>
            <i className={`fa-duotone fa-solid ${message.type === 'success' ? 'fa-circle-check duo-success' : 'fa-circle-exclamation duo-error'}`} />
            {message.text}
          </div>
        )}

        {/* Save */}
        <button
          onClick={handleSave}
          disabled={saving || !pathValid}
          className={`rounded-xl px-5 py-2.5 text-xs font-semibold transition-all flex items-center gap-2 action-btn action-btn-success
            ${saving
              ? 'bg-surface-container-high text-on-surface-variant border border-outline-variant/40 cursor-not-allowed opacity-70'
              : 'bg-success/12 text-on-surface border border-success/20 hover:bg-success/24 hover:border-success/40'
            }`}
        >
          {saving
            ? <><i className="fa-duotone fa-solid fa-spinner fa-spin" />Saving...</>
            : <><i className="fa-duotone fa-solid fa-floppy-disk icon-gradient icon-gradient-spring settings-save-icon" />Save Configuration</>
          }
        </button>
      </div>
      )}
    </div>
  )
}
