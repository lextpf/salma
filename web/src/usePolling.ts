import { useEffect, useRef } from 'react';

/**
 * Calls `fetcher` immediately when `enabled` turns true, then every
 * `intervalMs` milliseconds while it stays true. Clears the interval on unmount
 * and when `enabled` turns false.
 *
 * The fetcher is stored in a ref, so callers need not memoize it and a changed
 * fetcher reference does not restart the interval. A fetcher closing over state
 * therefore reads the latest values on the next tick, with the timing undisturbed.
 *
 * Calls never overlap: a tick that arrives while the previous call is still
 * pending is dropped, not queued. A fetcher slower than `intervalMs` runs less
 * often than the interval asks for, which is the intended trade.
 *
 * A rejected fetcher is caught and logged as a warning, never rethrown, so a
 * failing poll cannot break the render tree. Report failure inside the fetcher
 * if the UI has to show it.
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
