import { useRef, useState } from 'react'

interface FileUploadProps {
  onFileSelect: (files: FileList) => void
  disabled?: boolean
}

export default function FileUpload({ onFileSelect, disabled = false }: FileUploadProps) {
  const fileInputRef = useRef<HTMLInputElement>(null)
  const [isDragging, setIsDragging] = useState(false)

  const handleFileChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    if (disabled) return
    if (e.target.files && e.target.files.length > 0) {
      onFileSelect(e.target.files)
    }
  }

  const handleDragOver = (e: React.DragEvent) => {
    e.preventDefault()
    if (disabled) return
    setIsDragging(true)
  }

  const handleDragLeave = (e: React.DragEvent) => {
    e.preventDefault()
    if (disabled) return
    setIsDragging(false)
  }

  const handleDrop = (e: React.DragEvent) => {
    e.preventDefault()
    if (disabled) return
    setIsDragging(false)
    if (e.dataTransfer.files && e.dataTransfer.files.length > 0) {
      onFileSelect(e.dataTransfer.files)
    }
  }

  return (
    <div
      className={`group relative rounded-2xl p-8 text-center
        transition-colors duration-200 overflow-hidden upload-border
        ${disabled
          ? 'cursor-not-allowed bg-surface-container-high/55 border-outline-variant/55 opacity-90'
          : 'cursor-pointer'
        }
        ${isDragging && !disabled
          ? 'upload-border-active bg-primary/[0.04]'
          : (disabled ? '' : 'hover:bg-primary/[0.02]')
        }`}
      onDragOver={handleDragOver}
      onDragLeave={handleDragLeave}
      onDrop={handleDrop}
      onClick={() => { if (!disabled) fileInputRef.current?.click() }}
    >
      {/* Static aurora gradient bg */}
      <div className="absolute inset-0 bg-gradient-to-br from-primary/[0.035] via-transparent to-primary/[0.02] pointer-events-none" />
      {disabled && <div className="absolute inset-0 rounded-2xl bg-surface-container-highest/[0.25] pointer-events-none" />}
      <div className="absolute inset-0 pointer-events-none rounded-2xl border border-outline-variant/20" />
      <div className="absolute inset-0 pointer-events-none rounded-2xl upload-corner-soften" />

      <input
        ref={fileInputRef}
        type="file"
        multiple
        accept=".001,.7z,.fomod,.zip,.rar,.json"
        onChange={handleFileChange}
        disabled={disabled}
        className="hidden"
      />
      <div className="relative flex flex-col items-center gap-3">
        <div className={`rounded-2xl p-4 border shadow-elevation-1 ${
          disabled
            ? 'bg-surface-container-high border-outline-variant/40'
            : 'bg-gradient-to-br from-primary/14 to-primary/7 border-outline-variant/20'
        }`}>
          <i className={`fa-duotone fa-solid ${
            disabled
              ? 'fa-lock text-error/70'
              : (isDragging ? 'fa-box-open text-primary' : 'fa-conveyor-belt-boxes')
          } text-3xl ${disabled ? '' : `icon-gradient ${isDragging ? 'icon-gradient-atlas' : 'icon-gradient-nebula'}`}`} />
        </div>
        <div>
          <p className={`text-[1.02rem] font-semibold tracking-tight ${disabled ? 'text-error/75' : 'text-on-surface'}`}>
            {disabled ? 'Upload disabled until MO2 plugin is deployed' : (isDragging ? 'Drop files here' : 'Click or drag files to upload')}
          </p>
          <p className={`text-sm mt-1.5 ${disabled ? 'text-on-surface-variant/80' : 'text-on-surface-variant'}`}>
            Supports
            {' '}<span className={disabled ? 'text-on-surface-variant/80 font-semibold' : 'text-primary font-semibold'}>.7z</span>,
            {' '}<span className={disabled ? 'text-on-surface-variant/80 font-semibold' : 'text-secondary font-semibold'}>.zip</span>,
            {' '}<span className={disabled ? 'text-on-surface-variant/80 font-semibold' : 'text-warning font-semibold'}>.rar</span>,
            {' '}<span className={disabled ? 'text-on-surface-variant/80 font-semibold' : 'text-tertiary font-semibold'}>.fomod</span>,
            {' '}and <span className={disabled ? 'text-on-surface-variant/80 font-semibold' : 'text-info font-semibold'}>.json</span> files
          </p>
        </div>
      </div>
    </div>
  )
}
