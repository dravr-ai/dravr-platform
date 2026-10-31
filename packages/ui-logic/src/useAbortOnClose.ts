// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Hands a window's requests an AbortSignal that fires when the window closes
// ABOUTME: Closing a provider sign-in ends its login on the server instead of leaving it running

import { useCallback, useEffect, useRef } from 'react';

/**
 * Signals for the requests a window starts, aborted when it closes.
 *
 * Each call of the returned function aborts the request the window started
 * before it and hands out a fresh signal, so one window has at most one
 * request in flight. Closing the window (`open` turning false) or unmounting
 * it aborts that request: the connection drops, and the server drops the
 * handler that was waiting on it.
 *
 * A caller checks `signal.aborted` before touching its state once the request
 * settles, since an aborted request rejects after the window has moved on.
 */
export function useAbortOnClose(open: boolean): () => AbortSignal {
  const current = useRef<AbortController | null>(null);

  const abortCurrent = useCallback(() => {
    current.current?.abort();
    current.current = null;
  }, []);

  useEffect(() => {
    if (!open) abortCurrent();
  }, [open, abortCurrent]);

  useEffect(() => abortCurrent, [abortCurrent]);

  return useCallback(() => {
    abortCurrent();
    const controller = new AbortController();
    current.current = controller;
    return controller.signal;
  }, [abortCurrent]);
}
