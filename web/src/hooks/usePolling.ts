import { useEffect, useRef } from 'react';

/**
 * Polls a fetcher function at a fixed interval while enabled.
 * Automatically cleans up on unmount or when disabled.
 * Prevents overlapping requests via an inflight guard.
 *
 * The fetcher is stored in a ref so callers do not need to memoize it --
 * changing the fetcher reference will NOT restart the interval.
 * An immediate first call is made when the hook becomes enabled.
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

    poll();
    const id = setInterval(poll, intervalMs);
    return () => clearInterval(id);
  }, [intervalMs, enabled]);
}
