// While the instance is unreachable at launch, the tab tries again by itself, so one left open
// during a restart or an outage comes back without a click.

const MINIMUM_RETRY_DELAY_MILLISECONDS = 15_000;
const MAXIMUM_RETRY_DELAY_MILLISECONDS = 30_000;

/**
 * Calls `retry` when the window goes back online and after a delay between 15 and 30 s, drawn
 * anew each time so every tab does not hit a restarting instance at once; never two at a time.
 * Returns the function that stops it.
 */
export function retryWhenOnlineAndPeriodically(retry: () => Promise<void>): () => void {
  let timer: ReturnType<typeof setTimeout> | null = null;
  let isRetrying = false;
  let isStopped = false;
  const onlineListener = new AbortController();

  function scheduleRetry(): void {
    const spread = MAXIMUM_RETRY_DELAY_MILLISECONDS - MINIMUM_RETRY_DELAY_MILLISECONDS;
    timer = setTimeout(retryNow, MINIMUM_RETRY_DELAY_MILLISECONDS + Math.random() * spread);
  }

  function retryNow(): void {
    if (isRetrying) {
      return;
    }
    if (timer) {
      clearTimeout(timer);
      timer = null;
    }
    isRetrying = true;
    void retry().finally(() => {
      isRetrying = false;
      if (!isStopped) {
        scheduleRetry();
      }
    });
  }

  window.addEventListener("online", retryNow, { signal: onlineListener.signal });
  scheduleRetry();
  return () => {
    isStopped = true;
    onlineListener.abort();
    if (timer) {
      clearTimeout(timer);
      timer = null;
    }
  };
}
