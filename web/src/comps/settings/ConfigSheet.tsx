import type { ReactNode } from 'react'
import MIcon from '../MIcon'
import type { AppConfig } from '../../types'

const MONO = 'var(--font-mono)'

// Width of the key column (in monospace chars) before the `= `, so that
// mods_path / output / args / bind all align on the equals sign.
const KEY_COL = 11

function keyPad(key: string): string {
  return ' '.repeat(Math.max(KEY_COL - key.length, 1))
}

interface EditableFieldProps {
  value: string
  onChange: (v: string) => void
  placeholder: string
  ariaLabel: string
}

// An input styled to sit inside the mono code sheet: transparent, no chrome
// except a faint dashed underline that firms up on focus. Width tracks content
// length so it reads like a value typed straight into the file.
function EditableField({ value, onChange, placeholder, ariaLabel }: EditableFieldProps) {
  const chars = Math.max((value.length || placeholder.length) + 1, 10)
  return (
    <input
      type="text"
      aria-label={ariaLabel}
      value={value}
      placeholder={placeholder}
      spellCheck={false}
      autoComplete="off"
      onChange={(e) => { onChange(e.target.value); }}
      style={{
        fontFamily: MONO,
        fontSize: 'var(--fs-body)',
        fontWeight: 500,
        color: 'var(--ink)',
        background: 'transparent',
        border: 'none',
        borderBottom: '1px dashed var(--rule-strong)',
        padding: '0 2px',
        margin: 0,
        outline: 'none',
        width: `${chars}ch`,
        minWidth: 56,
        verticalAlign: 'baseline',
      }}
      onFocus={(e) => {
        e.currentTarget.style.background = 'var(--card-2)'
        e.currentTarget.style.borderBottomColor = 'var(--ink)'
      }}
      onBlur={(e) => {
        e.currentTarget.style.background = 'transparent'
        e.currentTarget.style.borderBottomColor = 'var(--rule-strong)'
      }}
    />
  )
}

interface ConfigSheetProps {
  config: AppConfig
  modsPath: string
  onModsPathChange: (v: string) => void
  testArgs: string
  onTestArgsChange: (v: string) => void
  valid: boolean
  saving: boolean
  onSave: () => void
  saveMessage: { type: 'success' | 'error'; text: string } | null
}

// The salma.json config sheet: a line-numbered mono "file" whose values are
// editable inline. Presentation only - all load/save state lives in the page.
export default function ConfigSheet({
  config,
  modsPath,
  onModsPathChange,
  testArgs,
  onTestArgsChange,
  valid,
  saving,
  onSave,
  saveMessage,
}: ConfigSheetProps) {
  const lines: ReactNode[] = []

  // 1 - comment
  lines.push(
    <span style={{ color: 'var(--ink-5)' }}># MO2 instance - edited live, written atomically</span>,
  )
  // 2 - [mo2]
  lines.push(<span style={{ color: 'var(--ink)', fontWeight: 600 }}>[mo2]</span>)
  // 3 - mods_path (editable) + inline validation badge
  lines.push(
    <>
      <span style={{ color: 'var(--ink-4)' }}>mods_path</span>
      <span>{`${keyPad('mods_path')}= `}</span>
      <EditableField
        value={modsPath}
        onChange={onModsPathChange}
        placeholder="D:\MO2\mods"
        ariaLabel="MO2 mods path"
      />
      <span
        style={{
          marginLeft: 10,
          fontSize: 'var(--fs-micro)',
          color: config.mo2ModsPathValid ? 'var(--ink)' : 'var(--ochre)',
        }}
      >
        <MIcon
          name={config.mo2ModsPathValid ? 'check_circle' : 'warning'}
          size={11}
          style={{ marginRight: 4, verticalAlign: '-0.15em' }}
        />
        {config.mo2ModsPathValid ? 'exists - directory' : 'not found - not a directory'}
      </span>
    </>,
  )
  // 4 - output (derived, read-only)
  if (config.fomodOutputDir) {
    lines.push(
      <span style={{ color: 'var(--ink-5)' }}>
        output{`${keyPad('output')}= `}
        {config.fomodOutputDir}
        <span style={{ marginLeft: 12, fontSize: 'var(--fs-micro)' }}># derived - read-only</span>
      </span>,
    )
  }
  // 5 - blank
  lines.push(<span>{' '}</span>)
  // 6 - [test]
  lines.push(<span style={{ color: 'var(--ink)', fontWeight: 600 }}>[test]</span>)
  // 7 - args (editable, persisted to localStorage by the page)
  lines.push(
    <>
      <span style={{ color: 'var(--ink-4)' }}>args</span>
      <span>{`${keyPad('args')}= `}</span>
      <EditableField
        value={testArgs}
        onChange={onTestArgsChange}
        placeholder={'--separator "My Separator" --limit 10'}
        ariaLabel="Test runner arguments"
      />
    </>,
  )
  // 8 - blank
  lines.push(<span>{' '}</span>)
  // 9 - [server]
  lines.push(<span style={{ color: 'var(--ink)', fontWeight: 600 }}>[server]</span>)
  // 10 - bind (read-only, loopback note)
  lines.push(
    <>
      <span style={{ color: 'var(--ink-4)' }}>bind</span>
      <span>{`${keyPad('bind')}= `}</span>
      <span style={{ color: 'var(--ink)' }}>127.0.0.1:5000</span>
      <span style={{ marginLeft: 10, fontSize: 'var(--fs-micro)', color: 'var(--ochre)' }}>
        <MIcon name="warning" size={11} style={{ marginRight: 4, verticalAlign: '-0.15em' }} />
        loopback only
      </span>
    </>,
  )

  const saveDisabled = saving || !valid

  return (
    <div>
      <div
        style={{
          border: '1px solid var(--rule)',
          borderRadius: 11,
          overflow: 'hidden',
          background: 'var(--sheet)',
          boxShadow: 'var(--shadow-elevation-2)',
        }}
      >
        {/* Card header: filename + validity + Save */}
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
          <span style={{ fontFamily: MONO, fontSize: 'var(--fs-label)', color: 'var(--ink-2)' }}>
            salma.json
          </span>
          <span style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
            <span
              style={{
                display: 'inline-flex',
                alignItems: 'center',
                gap: 6,
                fontFamily: MONO,
                fontSize: 'var(--fs-micro)',
                color: valid ? 'var(--ink)' : 'var(--danger)',
              }}
            >
              <span
                style={{
                  width: 6,
                  height: 6,
                  borderRadius: '50%',
                  background: valid ? 'var(--ink)' : 'var(--danger)',
                }}
              />
              {valid ? 'valid' : 'invalid'}
            </span>
            <button
              type="button"
              onClick={onSave}
              disabled={saveDisabled}
              style={{
                display: 'inline-flex',
                alignItems: 'center',
                gap: 6,
                padding: '6px 13px',
                borderRadius: 7,
                background: 'var(--ink)',
                color: 'var(--paper)',
                border: '1px solid var(--ink)',
                fontFamily: 'inherit',
                fontSize: 'var(--fs-label)',
                fontWeight: 600,
                cursor: saveDisabled ? 'not-allowed' : 'pointer',
                opacity: saveDisabled ? 0.5 : 1,
                transition: 'background-color 160ms ease',
              }}
              onMouseEnter={(e) => {
                if (!saveDisabled) {
                  e.currentTarget.style.background = 'var(--ink-2)'
                }
              }}
              onMouseLeave={(e) => {
                e.currentTarget.style.background = 'var(--ink)'
              }}
            >
              {saving ? (
                <MIcon name="progress_activity" className="m-spin" size={12} />
              ) : (
                <MIcon name="save" size={12} />
              )}
              <span>{saving ? 'Saving...' : 'Save'}</span>
            </button>
          </span>
        </div>

        {/* Code sheet: line-number gutter + mono content rows */}
        <div style={{ display: 'flex', fontFamily: MONO, fontSize: 'var(--fs-body)' }}>
          <div
            style={{
              width: 46,
              flexShrink: 0,
              padding: '15px 12px 15px 0',
              textAlign: 'right',
              color: 'var(--ink-faint)',
              borderRight: '1px solid var(--rule-faint)',
              userSelect: 'none',
            }}
          >
            {lines.map((_, i) => (
              <div key={i} style={{ height: 26, lineHeight: '26px' }}>
                {i + 1}
              </div>
            ))}
          </div>
          <div
            style={{
              flex: 1,
              minWidth: 0,
              padding: '15px 0 15px 18px',
              color: 'var(--ink-2)',
              overflowX: 'auto',
            }}
          >
            {lines.map((line, i) => (
              <div key={i} style={{ height: 26, lineHeight: '26px', whiteSpace: 'pre' }}>
                {line}
              </div>
            ))}
          </div>
        </div>
      </div>

      {/* Transient save result */}
      {saveMessage && (
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 6,
            marginTop: 13,
            fontFamily: MONO,
            fontSize: 'var(--fs-micro)',
            color: saveMessage.type === 'success' ? 'var(--moss)' : 'var(--danger)',
          }}
        >
          <span
            style={{
              width: 6,
              height: 6,
              borderRadius: '50%',
              background: saveMessage.type === 'success' ? 'var(--moss)' : 'var(--danger)',
            }}
          />
          <span>{saveMessage.text}</span>
        </div>
      )}

      {/* Static info footer */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          marginTop: 13,
          fontFamily: MONO,
          fontSize: 'var(--fs-micro)',
          color: 'var(--ink-5)',
        }}
      >
        <MIcon name="info" size={12} />
        <span>
          Fields validate inline - the FOMOD output path is derived from mods_path and can't be
          edited directly.
        </span>
      </div>
    </div>
  )
}
