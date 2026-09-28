// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Reads a provider's connection state from Dravr's get_connection_status answer
// ABOUTME: Polls it after a provider authorization opens, until connected, out of budget, or cancelled

/** How often connect_provider asks Dravr whether the provider it opened is connected yet. */
export const PROVIDER_STATUS_POLL_INTERVAL_MS = 2000;

/** How a wait on a provider's connection ended. */
export type ProviderConnectionWait =
  /** Dravr reports the provider connected. */
  | "connected"
  /** The budget ran out with the provider still unconnected: the page may still complete. */
  | "pending"
  /** The MCP host cancelled the request the wait belonged to. */
  | "cancelled";

/**
 * Whether a `get_connection_status` answer for one provider says it is connected and
 * usable. The tool answers `{ provider, status, connected, needs_reauth, backend }` for a
 * named provider; a connection that needs re-authorizing is not one a flow has to wait
 * for, so it reads as unconnected, as does any answer without that shape.
 */
export function isProviderConnected(result: unknown): boolean {
  const structured = (result as { structuredContent?: unknown } | null)?.structuredContent;
  if (typeof structured !== "object" || structured === null) {
    return false;
  }
  const status = structured as { connected?: unknown; needs_reauth?: unknown };
  return status.connected === true && status.needs_reauth !== true;
}

/** What one poll gets: its budget, its cadence, the host's cancellation, a log line sink. */
export interface ProviderConnectionPollOptions {
  waitMs: number;
  intervalMs: number;
  signal?: AbortSignal;
  log: (message: string) => void;
}

/**
 * Asks `readConnected` until it answers true, the budget is spent, or `signal` aborts.
 *
 * Every read gets a signal that aborts at whichever comes first of the budget's end and
 * the host's cancellation, so a status request that hangs cannot hold the wait past its
 * budget. A read that fails is logged and asked again at the next interval: one dropped
 * request says nothing about the flow. Nothing outlives the returned promise - the sleep
 * between reads is cleared when it ends, whichever way it ends.
 */
export async function pollProviderConnection(
  readConnected: (signal: AbortSignal) => Promise<boolean>,
  { waitMs, intervalMs, signal, log }: ProviderConnectionPollOptions,
): Promise<ProviderConnectionWait> {
  const deadline = Date.now() + waitMs;
  const interval = Math.max(1, intervalMs);

  for (;;) {
    if (signal?.aborted) {
      return "cancelled";
    }
    const remaining = deadline - Date.now();
    if (remaining <= 0) {
      return "pending";
    }

    const budget = AbortSignal.timeout(remaining);
    const readSignal = signal ? AbortSignal.any([signal, budget]) : budget;
    try {
      if (await readConnected(readSignal)) {
        return "connected";
      }
    } catch (error: any) {
      if (signal?.aborted) {
        return "cancelled";
      }
      log(`Connection status read failed, asking again: ${error?.message ?? error}`);
    }

    const pause = Math.min(interval, deadline - Date.now());
    if (pause > 0 && !(await sleep(pause, signal))) {
      return "cancelled";
    }
  }
}

/** Waits `ms`, resolving false as soon as `signal` aborts and true otherwise. */
function sleep(ms: number, signal?: AbortSignal): Promise<boolean> {
  if (signal?.aborted) {
    return Promise.resolve(false);
  }
  return new Promise((resolve) => {
    const onAbort = (): void => {
      clearTimeout(timer);
      resolve(false);
    };
    const timer = setTimeout(() => {
      signal?.removeEventListener("abort", onAbort);
      resolve(true);
    }, ms);
    signal?.addEventListener("abort", onAbort, { once: true });
  });
}
