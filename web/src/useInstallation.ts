import { useState, useRef, useEffect } from 'react'
import { getCsrfToken, getInstallStatus } from './api'
import type { InstallationJob } from './types'

export function useInstallation(pluginInstalled: boolean): {
  jobs: InstallationJob[]
  isInstalling: boolean
  handleFileSelect: (files: FileList) => Promise<void>
} {
  const [jobs, setJobs] = useState<InstallationJob[]>([])
  const [isInstalling, setIsInstalling] = useState(false)
  const cancelledRef = useRef(false)
  const xhrRef = useRef<XMLHttpRequest | null>(null)
  const pollTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

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

      // Fetch CSRF token before opening the XHR. Done synchronously here
      // (not inside the Promise constructor) so the token cache lookup is
      // awaited and any failure surfaces as the caller's catch.
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
          ? { ...j, status: 'processing', uploadProgress: 100, processingStatus: 'Installing mod...' }
          : j
      ))

      if (cancelledRef.current) return

      // Poll for completion using sequential setTimeout (no overlapping fetches).
      // The timer ID is tracked in pollTimerRef so cleanup on unmount can clear it.
      await new Promise<void>((resolve) => {
        const MAX_RETRIES = 200
        let retries = 0

        const poll = async () => {
          pollTimerRef.current = null
          if (cancelledRef.current) { resolve(); return }

          retries++
          if (retries > MAX_RETRIES) {
            setJobs(prev => prev.map(j =>
              j.id === job.id
                ? { ...j, status: 'error', error: 'Installation polling timed out after 5 minutes' }
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
                    ? { ...j, status: 'completed', modPath: status.modPath || result.modPath, processingStatus: 'Installation complete' }
                    : j
                ))
              } else {
                setJobs(prev => prev.map(j =>
                  j.id === job.id
                    ? { ...j, status: 'error', error: status.error || 'Installation failed' }
                    : j
                ))
              }
              resolve()
              return
            }
          } catch (e) {
            console.error('[install] transient error while polling install status', e)
          }

          // Schedule next poll only after current one completes (prevents overlap)
          pollTimerRef.current = setTimeout(poll, 1500)
        }

        if (cancelledRef.current) { resolve(); return }
        pollTimerRef.current = setTimeout(poll, 1500)
      })
    } catch (error) {
      setJobs(prev => prev.map(j =>
        j.id === job.id
          ? { ...j, status: 'error', error: error instanceof Error ? error.message : 'Unknown error' }
          : j
      ))
    }
  }

  const handleFileSelect = async (files: FileList) => {
    if (!pluginInstalled || isInstalling) return
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

  return { jobs, isInstalling, handleFileSelect }
}
