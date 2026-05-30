import { Fragment, useState, type ReactNode } from 'react'
import MIcon from './MIcon'
import { deriveProfile } from '../useChromeStatus'
import { useTheme, type ThemeMode } from '../theme'
import type { AppConfig } from '../types'

const MONO = 'var(--font-mono)'

interface ConfigSheetProps {
  config: AppConfig
  modsPath: string
  onModsPathChange: (v: string) => void
  testArgs: string
  onTestArgsChange: (v: string) => void
  tailLogs: boolean
  onTailLogsChange: (v: boolean) => void
  valid: boolean
  saveMessage: { type: 'success' | 'error'; text: string } | null
}

/**
 * A section break in the settings table.
 *
 * A tracked label on the page plane, not a card header: the sheet is one
 * continuous table rather than a stack of boxes. The note on the right says
 * where the section's values actually live, which is the question every one of
 * these rows is really answering.
 */
function SectionBand({ title, note }: { title: string; note: string }) {
  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 10,
        height: 26,
        // A section label has to belong to what follows it more than to what
        // precedes it. With no rule closing the previous section, only an
        // asymmetric gap can say that: wide above, none below. Do not even it
        // out.
        marginTop: 26,
        padding: '0 14px',
      }}
    >
      <h2
        style={{
          margin: 0,
          fontFamily: MONO,
          fontSize: 'var(--fs-micro)',
          fontWeight: 600,
          textTransform: 'uppercase',
          letterSpacing: 'var(--tr-kicker)',
          color: 'var(--ink-5)',
        }}
      >
        {title}
      </h2>
      <span style={{ flex: 1 }} />
      <span
        style={{
          fontFamily: MONO,
          fontSize: 'var(--fs-micro)',
          textTransform: 'uppercase',
          letterSpacing: 'var(--tr-chip)',
          // --ink-5, not --ink-6: the note carries a real value, and it holds
          // the same level as the section title beside it.
          color: 'var(--ink-5)',
          whiteSpace: 'nowrap',
        }}
      >
        {note}
      </span>
    </div>
  )
}

/** One table row: a label column and a control column. */
function Row({
  label,
  description,
  control,
}: {
  label: string
  description: string
  control: ReactNode
}) {
  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 20,
        padding: '12px 14px',
      }}
    >
      <div style={{ display: 'flex', flexDirection: 'column', gap: 3, flex: 1, minWidth: 0 }}>
        <div style={{ fontSize: 'var(--fs-title)', fontWeight: 600, letterSpacing: '-0.015em', color: 'var(--ink-2)' }}>
          {label}
        </div>
        <div style={{ fontSize: 'var(--fs-sm)', lineHeight: 1.5, color: 'var(--ink-5)', textWrap: 'pretty' }}>
          {description}
        </div>
      </div>
      <div style={{ flexShrink: 0, display: 'flex', alignItems: 'center', gap: 8 }}>{control}</div>
    </div>
  )
}

/** Flat input: fill plus a visible control border. Focus is the global outline. */
function TextField({
  value,
  onChange,
  placeholder,
  ariaLabel,
  width = 230,
}: {
  value: string
  onChange: (v: string) => void
  placeholder: string
  ariaLabel: string
  width?: number
}) {
  return (
    <input
      type="text"
      value={value}
      onChange={e => { onChange(e.target.value); }}
      placeholder={placeholder}
      aria-label={ariaLabel}
      spellCheck={false}
      style={{
        width,
        minWidth: 190,
        flexShrink: 0,
        height: 30,
        padding: '0 12px',
        border: '1px solid var(--rule-ctrl)',
        borderRadius: 'var(--radius-input)',
        background: 'var(--input)',
        color: 'var(--ink-2)',
        fontFamily: MONO,
        fontSize: 'var(--fs-mono)',
      }}
    />
  )
}

/** Chooser: the same flat geometry as the input, with the .btn hover states. */
function Select<T extends string>({
  value,
  options,
  onChange,
  ariaLabel,
}: {
  value: T
  options: { value: T; label: string }[]
  onChange: (v: T) => void
  ariaLabel: string
}) {
  // The real control is a transparent <select> laid over the chrome, and an
  // outline on a zero-opacity element paints nothing - so the global
  // :focus-visible ring never reached the user. Mirror it onto the visible
  // chrome, deferring to the browser's own :focus-visible decision so the ring
  // shows up exactly where it would on a native control.
  const [ring, setRing] = useState(false)
  return (
    <span
      className="btn"
      style={{
        position: 'relative',
        display: 'inline-flex',
        alignItems: 'center',
        gap: 9,
        flexShrink: 0,
        height: 30,
        padding: '0 10px 0 12px',
        border: '1px solid var(--rule-ctrl)',
        borderRadius: 'var(--radius-input)',
        background: 'var(--btn-bg)',
        color: 'var(--ink-2)',
        fontFamily: MONO,
        fontSize: 'var(--fs-mono)',
        cursor: 'pointer',
        outline: ring ? '2px solid var(--signal)' : undefined,
        outlineOffset: ring ? '1px' : undefined,
      }}
    >
      {options.find(o => o.value === value)?.label ?? value}
      <MIcon name="expand_more" size={15} style={{ color: 'var(--ink-4)' }} />
      {/* A real <select> laid over the chrome, transparent: native keyboard
          handling and the platform picker, our styling. */}
      <select
        value={value}
        onChange={e => { onChange(e.target.value as T); }}
        onFocus={e => setRing(e.currentTarget.matches(':focus-visible'))}
        onBlur={() => setRing(false)}
        aria-label={ariaLabel}
        style={{
          position: 'absolute',
          inset: 0,
          width: '100%',
          height: '100%',
          opacity: 0,
          cursor: 'pointer',
          font: 'inherit',
        }}
      >
        {options.map(o => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
    </span>
  )
}

/** 40x22 switch. Two flat fills and a flat knob: no inner lip, no bloom. */
function Toggle({ on, onChange, ariaLabel }: { on: boolean; onChange: (v: boolean) => void; ariaLabel: string }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={ariaLabel}
      onClick={() => onChange(!on)}
      style={{
        position: 'relative',
        width: 40,
        height: 22,
        flexShrink: 0,
        padding: 0,
        borderRadius: 'var(--radius-full)',
        border: `1px solid ${on ? 'var(--signal)' : 'var(--rule-ctrl)'}`,
        background: on ? 'var(--signal)' : 'var(--tog-off)',
        cursor: 'pointer',
      }}
    >
      <span
        aria-hidden="true"
        style={{
          position: 'absolute',
          top: 2,
          left: on ? 20 : 2,
          width: 16,
          height: 16,
          borderRadius: 'var(--radius-full)',
          background: on ? 'var(--signal-ink)' : 'var(--paper-3)',
          transition: 'left 200ms var(--ease)',
        }}
      />
    </button>
  )
}

/** Derived values are flat text, never a disabled input pretending to be one. */
function ReadOnly({ value, title }: { value: string; title?: string }) {
  return (
    <span
      title={title ?? value}
      style={{
        maxWidth: 260,
        fontFamily: MONO,
        fontSize: 'var(--fs-mono)',
        color: 'var(--ink-4)',
        whiteSpace: 'nowrap',
        overflow: 'hidden',
        textOverflow: 'ellipsis',
      }}
    >
      {value}
    </span>
  )
}

const THEME_OPTIONS: { value: ThemeMode; label: string }[] = [
  { value: 'system', label: 'System' },
  { value: 'dark', label: 'Dark' },
  { value: 'light', label: 'Light' },
]

/**
 * The settings sheet.
 *
 * One continuous table: section bands break it into groups and every setting is
 * a label and control row, so the screen reads as an instrument panel rather
 * than a stack of cards.
 *
 * Only rows backed by something real appear. The server persists exactly one
 * key, mo2ModsPath; the FOMOD output directory and the path-validity flag are
 * derived server-side, and the bind address comes from SALMA_BIND_ADDR in the
 * server's environment. Rows for solver strategy, log level and the like are
 * absent on purpose rather than rendered as controls that do nothing. The
 * Appearance and Test-harness groups are browser-local and label themselves so.
 *
 * The table closes on an environment readout whose lines look alike but are
 * three different kinds of value:
 *   - instance, mods root, path check and fomod output come from the saved
 *     server config, so they answer the editable rows above rather than echo
 *     them.
 *   - theme mode, tail logs and test args are browser-local preferences.
 *   - config file, persisted key, bind address and log file are literals
 *     written into this component. They state the default, not the resolved
 *     value. The bind address stays "127.0.0.1:5000" even once
 *     SALMA_BIND_ADDR has moved the real one, because /api/config does not
 *     carry it. Add it to AppConfig before treating that row as trustworthy.
 */
export default function ConfigSheet({
  config,
  modsPath,
  onModsPathChange,
  testArgs,
  onTestArgsChange,
  tailLogs,
  onTailLogsChange,
  valid,
  saveMessage,
}: ConfigSheetProps) {
  const { mode, setMode, theme } = useTheme()

  // An empty field is "not set yet", not "invalid": flagging a fresh install in
  // danger red reads as a fault when nothing has gone wrong.
  const pathState = modsPath.trim().length === 0
    ? { icon: 'remove', tone: 'var(--ink-5)', text: 'not set' }
    : !valid
      ? { icon: 'error', tone: 'var(--danger)', text: 'invalid path' }
      : config.mo2ModsPathValid
        ? { icon: 'check_circle', tone: 'var(--moss)', text: 'exists' }
        : { icon: 'warning', tone: 'var(--brass)', text: 'not found' }

  // The environment readout reports the saved path, not the edit buffer above.
  const resolvedState = config.mo2ModsPath.trim().length === 0
    ? { text: 'not set', tone: 'var(--ink-5)' }
    : config.mo2ModsPathValid
      ? { text: 'exists', tone: 'var(--moss)' }
      : { text: 'not found', tone: 'var(--brass)' }

  const env: { k: string; v: string; tone?: string }[] = [
    { k: 'instance', v: deriveProfile(config.mo2ModsPath) },
    { k: 'mods root', v: config.mo2ModsPath || 'not set' },
    { k: 'path check', v: resolvedState.text, tone: resolvedState.tone },
    { k: 'fomod output', v: config.fomodOutputDir || 'not derived yet' },
    { k: 'config file', v: 'salma.json' },
    { k: 'persisted key', v: 'mo2ModsPath' },
    // The compile-time default, not the resolved address: SALMA_BIND_ADDR can
    // move it and /api/config does not report where it landed.
    { k: 'bind address', v: '127.0.0.1:5000' },
    { k: 'log file', v: 'logs/salma.log' },
    { k: 'theme mode', v: mode === 'system' ? `system (${theme})` : mode },
    { k: 'tail logs', v: tailLogs ? 'on' : 'off' },
    { k: 'test args', v: testArgs.trim() || 'none' },
  ]

  return (
    <div style={{ display: 'flex', flexDirection: 'column' }}>
      <SectionBand title="Mod Organizer 2" note="salma.json" />
      <Row
        label="Mods directory"
        description="Root of the MO2 mods folder salma scans and writes into. The only value persisted to salma.json."
        control={
          <>
            <TextField
              value={modsPath}
              onChange={onModsPathChange}
              placeholder="D:/Games/MO2/mods"
              ariaLabel="MO2 mods directory"
            />
            <span
              title={pathState.text}
              style={{
                display: 'inline-flex',
                alignItems: 'center',
                gap: 5,
                fontFamily: MONO,
                fontSize: 'var(--fs-micro)',
                color: pathState.tone,
              }}
            >
              <MIcon name={pathState.icon} size={12} />
              {pathState.text}
            </span>
          </>
        }
      />
      <Row
        label="Active instance"
        description="Multi-instance support is planned; salma binds to one instance today, derived from the mods path."
        control={<ReadOnly value={deriveProfile(config.mo2ModsPath)} />}
      />
      <Row
        label="FOMOD output"
        description="Where inferred selection records are written. Derived from the mods path and not separately editable."
        control={<ReadOnly value={config.fomodOutputDir || 'not derived yet'} />}
      />

      <SectionBand title="Appearance" note="browser-local" />
      <Row
        label="Theme"
        description="System follows your OS setting and keeps following it. Stored in this browser."
        control={<Select value={mode} options={THEME_OPTIONS} onChange={setMode} ariaLabel="Theme" />}
      />
      <Row
        label="Tail logs by default"
        description="Whether module 03 starts following the log as it is written. Stored in this browser."
        control={<Toggle on={tailLogs} onChange={onTailLogsChange} ariaLabel="Tail logs by default" />}
      />

      <SectionBand title="Test harness" note="browser-local" />
      <Row
        label="Test arguments"
        description="Passed to the round-trip harness when you run tests. Stored in this browser only, not in salma.json."
        control={
          <TextField
            value={testArgs}
            onChange={onTestArgsChange}
            placeholder='--separator "My Separator" --limit 10'
            ariaLabel="Test runner arguments"
            width={280}
          />
        }
      />

      <SectionBand title="Server and diagnostics" note="read-only" />
      {/* The value shown is the default. It does not follow SALMA_BIND_ADDR,
          so it is wrong for exactly the user who acted on this description. */}
      <Row
        label="Listen address"
        description="mo2-server binds loopback only by default. Change it with the SALMA_BIND_ADDR environment variable; non-loopback values log a security warning. This row shows the default, not the address in use."
        control={<ReadOnly value="127.0.0.1:5000" />}
      />
      <Row
        label="Log file"
        description="Written next to the running module with 10 MiB rotation. Read it from module 03."
        control={<ReadOnly value="logs/salma.log" />}
      />

      {saveMessage && (
        <div
          role="status"
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 8,
            padding: '9px 14px',
            background: saveMessage.type === 'success' ? 'var(--ok-bg)' : 'var(--danger-wash)',
            fontFamily: MONO,
            fontSize: 'var(--fs-mono)',
            color: saveMessage.type === 'success' ? 'var(--moss)' : 'var(--danger)',
          }}
        >
          <span
            aria-hidden="true"
            style={{
              width: 6,
              height: 6,
              borderRadius: 'var(--radius-full)',
              background: saveMessage.type === 'success' ? 'var(--moss)' : 'var(--danger)',
            }}
          />
          {saveMessage.text}
        </div>
      )}

      <SectionBand title="Environment" note="resolved" />
      <div
        style={{
          display: 'grid',
          gridTemplateColumns: '116px minmax(0, 1fr)',
          columnGap: 14,
          rowGap: 4,
          padding: '11px 14px 13px',
        }}
      >
        {env.map(e => (
          <Fragment key={e.k}>
            <span
              style={{
                fontFamily: MONO,
                fontSize: 'var(--fs-micro)',
                textTransform: 'uppercase',
                letterSpacing: 'var(--tr-chip)',
                color: 'var(--ink-5)',
                whiteSpace: 'nowrap',
              }}
            >
              {e.k}
            </span>
            <span
              title={e.v}
              style={{
                fontFamily: MONO,
                fontSize: 'var(--fs-mono)',
                color: e.tone ?? 'var(--ink-3)',
                whiteSpace: 'nowrap',
                overflow: 'hidden',
                textOverflow: 'ellipsis',
              }}
            >
              {e.v}
            </span>
          </Fragment>
        ))}
      </div>
    </div>
  )
}
