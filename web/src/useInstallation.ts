/**
 * @brief Upload archives sequentially and poll each install to completion.
 * @author Alex (<https://github.com/lextpf>)
 *
 * Matching JSON files become multipart metadata. Unmatched JSON files are ignored.
 *
 * ### :material-timer-outline: Polling and cancellation
 *
 * Polling waits 1.5 seconds before each attempt and allows 200 status requests.
 * Request duration adds to this delay, so the limit is not a five-minute deadline.
 * Cancellation aborts an active upload and clears local polling timers.
 * Server-side work can continue.
 */
import { useState, useRef, useEffect, useCallback } from 'react'
import { getCsrfToken, getInstallStatus } from './api'
import type { InstallationJob } from './types'

/**
 * @fn useInstallation(pluginInstalled: boolean): {
 *   jobs: InstallationJob[]; isInstalling: boolean;
 *   handleFileSelect: (files: FileList) => Promise<void>; cancel: () => void
 * }
 * @brief Manage sequential archive uploads and installation polling.
 * @author Alex (<https://github.com/lextpf>)
 *
 * @param pluginInstalled Whether new file selections may start installation.
 * @return Job history, local activity state, file-selection handler, and cancellation callback.
 */
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
  // Settle the polling promise when cancellation clears its timer.
  const pollResolveRef = useRef<(() => void) | null>(null)
  /**
   * @fn isCancelled(): boolean
   * @brief Read the current cancellation flag across asynchronous boundaries.
   * @author Alex (<https://github.com/lextpf>)
   *
   * @return True after local cancellation or unmount.
   */
  const isCancelled = () => cancelledRef.current

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

  /**
   * @fn processJob(job: InstallationJob, file: File, jsonFile?: File): Promise<void>
   * @brief Upload one archive and record its observed installation result.
   * @author Alex (<https://github.com/lextpf>)
   *
   * Upload and polling failures update the job instead of rejecting the queue.
   * Completion comes from the server-wide status endpoint, which has no browser job ID.
   *
   * @param job Browser job whose state is updated by ID.
   * @param file Archive sent in multipart form data.
   * @param jsonFile Optional matching selection JSON, read as text.
   */
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

      if (isCancelled()) return

      // XHR provides upload progress. It has no timeout or CSRF retry.
      // Do not retry a large request body without an idempotency contract.
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
            modName: result.modName,
          }
          : j
      ))

      if (isCancelled()) return

      // Chain timers so status requests do not overlap.
      await new Promise<void>((resolve) => {
        pollResolveRef.current = resolve
        const MAX_RETRIES = 200
        let retries = 0

        /**
         * @fn poll(): Promise<void>
         * @brief Read install status and schedule the next attempt after it settles.
         * @author Alex (<https://github.com/lextpf>)
         */
        const poll = async () => {
          pollTimerRef.current = null
          if (isCancelled()) { resolve(); return }

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
            if (isCancelled()) { resolve(); return }
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

        if (isCancelled()) { resolve(); return }
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
              error: isCancelled()
                ? 'Cancelled'
                : error instanceof Error
                  ? error.message
                  : 'Unknown error',
            }
          : j
      ))
    }
  }

  /**
   * @fn handleFileSelect(files: FileList): Promise<void>
   * @brief Append a file selection and process its archives in order.
   * @author Alex (<https://github.com/lextpf>)
   *
   * Match JSON by archive stem without case sensitivity. Ignore unmatched JSON files.
   * Return without queuing when the plugin is unavailable or a batch is already active.
   *
   * @param files Archives and optional JSON files from one picker or drop event.
   */
  const handleFileSelect = async (files: FileList) => {
    if (!pluginInstalled || isInstalling) return
    // Re-arm after a prior cancellation.
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
        if (isCancelled()) break
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

  /**
   * @fn cancel(): void
   * @brief Stop local uploads and polling and mark unfinished jobs as cancelled.
   * @author Alex (<https://github.com/lextpf>)
   *
   * This callback does not request cancellation of server-side installation work.
   */
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
