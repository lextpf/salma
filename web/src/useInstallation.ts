/**
 * @brief upload archives sequentially and poll each install to completion.
 * @author Alex (https://github.com/lextpf)
 *
 * matching JSON files become multipart metadata. unmatched JSON files are ignored.
 *
 * ### :material-timer-outline: polling and cancellation
 *
 * polling waits 1.5 seconds before each attempt and stops after 200 attempts.
 * cancellation stops local upload and polling only. server-side work can continue.
 */
import { useState, useRef, useEffect, useCallback } from 'react'
import { getCsrfToken, getInstallStatus } from './api'
import type { InstallationJob } from './types'

export function useInstallation(pluginInstalled: boolean): {
  jobs: InstallationJob[]
  isInstalling: boolean
  handleFileSelect: (files: FileList) => Promise<void>
  cancel: () => void
} {
  const [jobs, setJobs] = useState<InstallationJob[]>([])
  const [isInstalling, setIsInstalling] = useState(false)
  const cancelledRef = useRef(false)
  const xhrRef = useRef<XMLHttpRequest | null>(null)
  const pollTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  // settle the polling promise when cancellation clears its timer.
  const pollResolveRef = useRef<(() => void) | null>(null)

  useEffect(() => {
    return () => {
      cancelledRef.current = true
      if (xhrRef.current) {
        xhrRef.current.abort()
        xhrRef.current = null
      }
      if (pollTimerRef.current) {
        clearTimeout(pollTimerRef.current)
        pollTimerRef.current = null
      }
      if (pollResolveRef.current) {
        pollResolveRef.current()
        pollResolveRef.current = null
      }
    }
  }, [])

  const processJob = async (job: InstallationJob, file: File, jsonFile?: File) => {
    setJobs(prev => prev.map(j => j.id === job.id ? { ...j, status: 'uploading', uploadProgress: 0 } : j))

    try {
      const formData = new FormData()
      formData.append('file', file)

      if (jsonFile) {
        const jsonText = await jsonFile.text()
        formData.append('fomodJson', jsonText)
        formData.append('jsonFileName', jsonFile.name)
      }

      if (cancelledRef.current) return

      // XHR provides upload progress. it has no timeout or CSRF retry.
      // do not retry a large request body without an idempotency contract.
      const csrfToken = await getCsrfToken()

      const result = await new Promise<Record<string, string>>((resolve, reject) => {
        const xhr = new XMLHttpRequest()
        xhrRef.current = xhr

        xhr.upload.addEventListener('progress', (e) => {
          if (e.lengthComputable) {
            const progress = (e.loaded / e.total) * 100
            setJobs(prev => prev.map(j =>
              j.id === job.id ? { ...j, uploadProgress: progress } : j
            ))
          }
        })

        xhr.addEventListener('load', () => {
          xhrRef.current = null
          if (xhr.status >= 200 && xhr.status < 300) {
            try {
              resolve(JSON.parse(xhr.responseText))
            } catch (e) {
              console.error('[install] failed to parse upload response as JSON', e)
              reject(new Error('Invalid JSON response'))
            }
          } else {
            let errorMessage = `HTTP ${xhr.status}: ${xhr.statusText}`
            try {
              const errorData = JSON.parse(xhr.responseText)
              errorMessage = errorData.error || errorMessage
            } catch (e) {
              console.error('[install] failed to parse error response as JSON', e)
              errorMessage = xhr.responseText || errorMessage
            }
            reject(new Error(errorMessage))
          }
        })

        xhr.addEventListener('error', () => { xhrRef.current = null; reject(new Error('Network error during upload')) })
        xhr.addEventListener('abort', () => { xhrRef.current = null; reject(new Error('Upload aborted')) })

        xhr.open('POST', '/api/installation/upload')
        xhr.setRequestHeader('X-Salma-Csrf', csrfToken)
        xhr.send(formData)
      })

      setJobs(prev => prev.map(j =>
        j.id === job.id
          ? {
            ...j,
            status: 'processing',
            uploadProgress: 100,
            processingStatus: 'Installing mod...',
            modName: result.modName ?? j.modName,
          }
          : j
      ))

      if (cancelledRef.current) return

      // chain timers so status requests do not overlap.
      await new Promise<void>((resolve) => {
        pollResolveRef.current = resolve
        const MAX_RETRIES = 200
        let retries = 0

        const poll = async () => {
          pollTimerRef.current = null
          if (cancelledRef.current) { resolve(); return }

          retries++
          if (retries > MAX_RETRIES) {
            setJobs(prev => prev.map(j =>
              j.id === job.id
                ? { ...j, status: 'error', completedAt: Date.now(), error: 'Installation polling timed out after 5 minutes' }
                : j
            ))
            resolve()
            return
          }

          try {
            const status = await getInstallStatus()
            if (cancelledRef.current) { resolve(); return }
            if (!status.running) {
              if (status.success) {
                setJobs(prev => prev.map(j =>
                  j.id === job.id
                    ? { ...j, status: 'completed', completedAt: Date.now(), modPath: status.modPath || result.modPath, processingStatus: 'Installation complete' }
                    : j
                ))
              } else {
                setJobs(prev => prev.map(j =>
                  j.id === job.id
                    ? { ...j, status: 'error', completedAt: Date.now(), error: status.error || 'Installation failed' }
                    : j
                ))
              }
              resolve()
              return
            }
          } catch (e) {
            console.error('[install] transient error while polling install status', e)
          }

          pollTimerRef.current = setTimeout(poll, 1500)
        }

        if (cancelledRef.current) { resolve(); return }
        pollTimerRef.current = setTimeout(poll, 1500)
      })
      pollResolveRef.current = null
    } catch (error) {
      setJobs(prev => prev.map(j =>
        j.id === job.id
          ? {
              ...j,
              status: 'error',
              completedAt: Date.now(),
              error: cancelledRef.current
                ? 'Cancelled'
                : error instanceof Error
                  ? error.message
                  : 'Unknown error',
            }
          : j
      ))
    }
  }

  const handleFileSelect = async (files: FileList) => {
    if (!pluginInstalled || isInstalling) return
    // re-arm after a prior cancellation.
    cancelledRef.current = false
    setIsInstalling(true)

    try {
      const fileArray = Array.from(files)

      const archiveFiles: File[] = []
      const jsonFiles: File[] = []

      fileArray.forEach(file => {
        const ext = file.name.toLowerCase().split('.').pop()
        if (ext === 'json') {
          jsonFiles.push(file)
        } else {
          archiveFiles.push(file)
        }
      })

      const newJobs: InstallationJob[] = archiveFiles.map(file => ({
        id: crypto.randomUUID(),
        fileName: file.name,
        status: 'pending' as const,
        createdAt: Date.now(),
        sizeBytes: file.size,
      }))

      setJobs(prev => [...prev, ...newJobs])

      for (let i = 0; i < newJobs.length; i++) {
        if (cancelledRef.current) break
        const archiveFile = archiveFiles[i]
        const archiveNameWithoutExt = archiveFile.name.replace(/\.[^/.]+$/, '')
        const matchingJson = jsonFiles.find(json =>
          json.name.toLowerCase() === `${archiveNameWithoutExt.toLowerCase()}.json`
        )
        await processJob(newJobs[i], archiveFile, matchingJson)
      }
    } finally {
      setIsInstalling(false)
    }
  }

  // cancellation does not stop server-side installation work.
  const cancel = useCallback(() => {
    cancelledRef.current = true
    if (xhrRef.current) {
      xhrRef.current.abort()
      xhrRef.current = null
    }
    if (pollTimerRef.current) {
      clearTimeout(pollTimerRef.current)
      pollTimerRef.current = null
    }
    if (pollResolveRef.current) {
      pollResolveRef.current()
      pollResolveRef.current = null
    }
    setJobs(prev => prev.map(j =>
      j.status === 'pending' || j.status === 'uploading' || j.status === 'processing'
        ? { ...j, status: 'error', completedAt: Date.now(), error: 'Cancelled' }
        : j
    ))
    setIsInstalling(false)
  }, [])

  return { jobs, isInstalling, handleFileSelect, cancel }
}
