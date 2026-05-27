import { useState, useRef, useEffect, useCallback } from 'react'
import { getCsrfToken, getInstallStatus } from './api'
import type { InstallationJob } from './types'

/**
 * Owns the Install screen's job list: upload, then poll to completion.
 *
 * The job state machine is drawn on InstallationJob in types.ts. Behaviour a
 * caller has to know:
 *
 *   - Jobs run strictly one at a time, in selection order, because the server
 *     holds a single install slot. handleFileSelect resolves only after the
 *     last job in the batch settles, and it returns early while isInstalling.
 *   - A dropped .json file is not a job. It is matched case-insensitively by
 *     stem to an archive in the same batch and sent with it as the multipart
 *     `fomodJson` field. An unmatched .json is silently ignored.
 *   - `cancel()` aborts the in-flight upload, stops polling and marks every
 *     non-terminal job as an error reading 'Cancelled'. It does not tell the
 *     server to stop: an install already running there runs to completion and
 *     writes its output. Cancellation stays armed until the next
 *     handleFileSelect re-arms it.
 *   - Polling gives up after 200 attempts at 1.5s, so about 5 minutes. A
 *     transient poll failure is logged and retried; only the attempt ceiling
 *     ends the job.
 *   - `pluginInstalled` false makes handleFileSelect a no-op, with no job row
 *     and no message. The caller must explain the refusal.
 *
 * Unmounting aborts the upload, clears the poll timer and settles the pending
 * poll promise, so nothing is left running. The jobs array is lost with it;
 * there is no session history beyond the component's lifetime.
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
  // Holds the resolver of the in-flight polling promise so cancel()/unmount can
  // settle it immediately after clearing the timer (otherwise it dangles).
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

      // Fetch the CSRF token before opening the XHR, and outside the Promise
      // constructor, so the cache lookup is awaited and a failure surfaces in
      // the caller's catch.
      //
      // This upload is the one call that does not go through api.ts, so it has
      // neither protection the rest of the app gets:
      //   - No CSRF retry. api.ts drops its cached token and resends once on a
      //     403 whose body is "csrf token missing or invalid". Here a stale
      //     token is final. The server rotates its token on every restart, so
      //     restarting mo2-server with the dashboard open fails the next upload
      //     with the raw 403 body as the job's error, while every other call
      //     recovers silently. Reloading the page clears it.
      //   - No timeout. XHR is here for its upload-progress events and no abort
      //     signal is armed, so an upload that never answers leaves the job in
      //     `uploading` until the user cancels.
      // Do not "simplify" this to fetch() without first replacing the progress
      // events, and do not add a retry that blindly resends a large body.
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

      // Poll for completion with a chained setTimeout, so two polls never
      // overlap. pollTimerRef holds the pending id for unmount cleanup.
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

          // The next poll is scheduled only once this one has finished.
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
    // Re-arm after a prior cancel so a fresh selection can run.
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

  // Abort the active install: stop the in-flight upload XHR, clear the poll
  // timer, settle the dangling poll promise, and flag every non-terminal job as
  // cancelled. cancelledRef stays true until the next handleFileSelect re-arms.
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
