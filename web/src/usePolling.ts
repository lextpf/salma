import { useEffect, useRef } from 'react';

/**
 * @fn usePolling(fetcher: () => Promise<void>, intervalMs: number, enabled: boolean): void
 * @brief Run a non-overlapping poll while enabled.
 * @author Alex (<https://github.com/lextpf>)
 *
 * The first call is immediate. Slow calls drop ticks. Rejected calls are logged and not rethrown.
 * Cleanup stops future ticks; it does not cancel an in-flight request. The fetcher must guard
 * state updates after unmount or a change of request identity.
 * @param fetcher The asynchronous poll operation.
 * @param intervalMs Interval in milliseconds.
 * @param enabled True while polling is active.
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

    /**
     * @fn poll(): Promise<void>
     * @brief Skip overlapping ticks and release the shared guard after each attempt.
     * @author Alex (<https://github.com/lextpf>)
     */
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
