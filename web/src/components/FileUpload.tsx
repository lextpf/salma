import { useRef, useState } from 'react'

interface FileUploadProps {
  onFileSelect: (files: FileList) => void
  disabled?: boolean
  purged?: boolean
}

const SUPPORTED_FORMATS: { ext: string; color: string }[] = [
  { ext: '.7z',    color: 'var(--accent)' },
  { ext: '.zip',   color: 'var(--ink-blue)' },
  { ext: '.rar',   color: 'var(--ochre)' },
  { ext: '.fomod', color: 'var(--moss)' },
  { ext: '.tar',   color: 'var(--ink-2)' },
]

export default function FileUpload({ onFileSelect, disabled = false, purged = false }: FileUploadProps) {
  const fileInputRef = useRef<HTMLInputElement>(null)
  const [isDragging, setIsDragging] = useState(false)

  const handleFileChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    if (disabled) return
    if (e.target.files && e.target.files.length > 0) onFileSelect(e.target.files)
  }
  const handleDragOver = (e: React.DragEvent) => {
    e.preventDefault()
    if (!disabled) setIsDragging(true)
  }
  const handleDragLeave = (e: React.DragEvent) => {
    e.preventDefault()
    if (!disabled) setIsDragging(false)
  }
  const handleDrop = (e: React.DragEvent) => {
    e.preventDefault()
    if (disabled) return
    setIsDragging(false)
    if (e.dataTransfer.files && e.dataTransfer.files.length > 0) onFileSelect(e.dataTransfer.files)
  }

  return (
    <div
      className="upload-dropzone"
      data-dragging={isDragging ? 'true' : 'false'}
      data-disabled={disabled ? 'true' : 'false'}
      data-purged={purged ? 'true' : 'false'}
      onDragOver={handleDragOver}
      onDragLeave={handleDragLeave}
      onDrop={handleDrop}
      onClick={() => { if (!disabled) fileInputRef.current?.click() }}
    >
      <input
        ref={fileInputRef}
        type="file"
        multiple
        accept=".001,.7z,.fomod,.zip,.rar,.tar,.gz,.bz2,.xz,.json"
        onChange={handleFileChange}
        disabled={disabled}
        className="hidden"
      />

      <div className="empty-state-card upload-dropzone-card" style={{ gap: 18 }}>
        {/* Feather icon in oxblood circle */}
        <div
          className="upload-dropzone-icon-ring"
          style={{
            color: purged ? 'var(--danger)' : disabled ? 'var(--ink-4)' : 'var(--accent)',
            transform: isDragging ? 'scale(1.05) rotate(-6deg)' : 'scale(1) rotate(0)',
          }}
        >
          <i
            className={`fa-duotone fa-solid ${disabled ? 'fa-lock' : isDragging ? 'fa-wand-magic-sparkles' : 'fa-feather-pointed'}`}
            style={{ fontSize: 16 }}
          />
        </div>

        <div style={{ flex: 1, minWidth: 0 }}>
          {/* Headline */}
          <p
            className="display-serif-italic"
            style={{
              fontSize: 22,
              lineHeight: 1.15,
              color: 'var(--ink)',
              margin: 0,
            }}
          >
            {disabled ? (
              <>Upload<span className="display-period">.</span> locked</>
            ) : isDragging ? (
              <>Drop it here<span className="display-period">.</span></>
            ) : (
              <>Click or drag to install<span className="display-period">.</span></>
            )}
          </p>

          {/* Format list - plain colored mono text per type */}
          {!disabled ? (
            <div className="flex items-baseline" style={{ gap: 10, marginTop: 6, flexWrap: 'wrap' }}>
              <span
                style={{
                  fontFamily: 'var(--font-mono)',
                  fontSize: 13,
                  letterSpacing: '0.04em',
                  color: 'var(--ink-4)',
                }}
              >
                // supports
              </span>
              <span
                className="flex items-baseline"
                style={{
                  fontFamily: 'var(--font-mono)',
                  fontSize: 13,
                  letterSpacing: '0.04em',
                  gap: 10,
                  flexWrap: 'wrap',
                }}
              >
                {SUPPORTED_FORMATS.map(f => (
                  <span key={f.ext} style={{ color: f.color, fontWeight: 500 }}>{f.ext}</span>
                ))}
              </span>
              <span style={{ color: 'var(--ink-5)' }}>-</span>
              <span
                className="serif-link-arrow"
                style={{
                  fontFamily: 'var(--font-mono)',
                  fontSize: 13,
                  letterSpacing: '0.04em',
                }}
              >
                browse <span className="arrow" aria-hidden="true">&rarr;</span>
              </span>
            </div>
          ) : (
            <p
              className="timestamp-print"
              style={{ marginTop: 4, color: 'var(--ink-4)' }}
            >
              // deploy the MO2 plugin to unlock uploads
            </p>
          )}
        </div>
      </div>
    </div>
  )
}
