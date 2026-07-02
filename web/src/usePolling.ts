import { useEffect, useRef } from 'react';

/**
 * @fn usePolling(fetcher: () => Promise<void>, intervalMs: number, enabled: boolean): void
 * @brief run a non-overlapping poll while enabled.
 * @author Alex (https://github.com/lextpf)
 *
 * the first call is immediate. slow calls drop ticks. rejected calls are logged and not rethrown.
 * @param fetcher the asynchronous poll operation.
 * @param intervalMs interval in milliseconds.
 * @param enabled true while polling is active.
 */
export function usePolling(
  fetcher: () => Promise<void>,
  intervalMs: number,
  enabled: boolean,
): void {
  const inFlightRef = useRef(false);
  const fetcherRef = useRef(fetcher);
  useEffect(() => { fetcherRef.current = fetcher }, [fetcher]);

  useEffect(() => {
    if (!enabled) return;

    const poll = async () => {
      if (inFlightRef.current) return;
      inFlightRef.current = true;
      try {
        await fetcherRef.current();
      } catch (e) {
        console.warn('[usePolling] error:', e);
      } finally {
        inFlightRef.current = false;
      }
    };

    void poll();
    const id = setInterval(poll, intervalMs);
    return () => { clearInterval(id); };
  }, [intervalMs, enabled]);
}
