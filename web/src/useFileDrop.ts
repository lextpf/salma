import { useCallback, useRef, useState } from 'react'

interface UseFileDropOptions {
  onFiles: (files: FileList) => void
  disabled?: boolean
}

export interface FileDrop {
  isDragging: boolean
  inputRef: React.RefObject<HTMLInputElement>
  openPicker: () => void
  onInputChange: (e: React.ChangeEvent<HTMLInputElement>) => void
  onDragOver: (e: React.DragEvent) => void
  onDragLeave: (e: React.DragEvent) => void
  onDrop: (e: React.DragEvent) => void
}

/**
 * @fn useFileDrop({ onFiles, disabled = false }: UseFileDropOptions): FileDrop
 * @brief combine picker and drag input without hiding repeated selections.
 * @author Alex (https://github.com/lextpf)
 *
 * callers must attach `inputRef` and `onInputChange` to one file input.
 */
export function useFileDrop({ onFiles, disabled = false }: UseFileDropOptions): FileDrop {
  const inputRef = useRef<HTMLInputElement>(null)
  const [isDragging, setIsDragging] = useState(false)

  const openPicker = useCallback(() => {
    if (!disabled) inputRef.current?.click()
  }, [disabled])

  const onInputChange = useCallback((e: React.ChangeEvent<HTMLInputElement>) => {
    if (disabled) return
    const files = e.target.files
    if (files && files.length > 0) onFiles(files)
    // allow the same file to trigger a later change event.
    e.target.value = ''
  }, [disabled, onFiles])

  const onDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault()
    if (!disabled) setIsDragging(true)
  }, [disabled])

  const onDragLeave = useCallback((e: React.DragEvent) => {
    e.preventDefault()
    if (!disabled) setIsDragging(false)
  }, [disabled])

  const onDrop = useCallback((e: React.DragEvent) => {
    e.preventDefault()
    if (disabled) return
    setIsDragging(false)
    const files = e.dataTransfer.files
    if (files && files.length > 0) onFiles(files)
  }, [disabled, onFiles])

  return { isDragging, inputRef, openPicker, onInputChange, onDragOver, onDragLeave, onDrop }
}
