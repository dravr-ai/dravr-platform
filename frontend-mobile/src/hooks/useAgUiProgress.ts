// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
// ABOUTME: Mobile hook that subscribes to AG-UI SSE and surfaces per-event progress status text
// ABOUTME: Uses react-native-sse EventSource with Bearer JWT against /api/agui/runs/{run_id}/stream

import { useEffect, useRef, useState } from 'react';
import EventSource from 'react-native-sse';
import * as SecureStore from 'expo-secure-store';
import { Platform } from 'react-native';
import {
  statusTextForAguiEvent,
  isTerminalAguiEvent,
  type AguiEventWire,
} from '../utils/aguiStatus';

const JWT_KEY = 'pierre.jwt_token';

/**
 * Resolve the AG-UI base URL the same way `services/api/index.ts`
 * resolves it for REST — if the shape ever diverges the mobile app
 * will surface a clean "no progress" fall-through rather than panic.
 */
function getApiUrl(): string {
  if (process.env.EXPO_PUBLIC_API_URL) {
    return process.env.EXPO_PUBLIC_API_URL;
  }
  if (Platform.OS === 'android') {
    return 'http://10.0.2.2:8081';
  }
  return 'http://localhost:8081';
}

/**
 * Shape returned by {@link useAgUiProgress}.
 */
export interface AgUiProgressState {
  /**
   * Current status text rendered to the user, e.g. "reading your
   * question…". `null` when no progress is known yet (pre-connect)
   * or after a terminal event.
   */
  statusText: string | null;
  /** `true` between `RUN_STARTED` and `RUN_FINISHED`/`RUN_ERROR`. */
  isActive: boolean;
}

/**
 * Subscribe to AG-UI progress events for `runId` and expose the
 * latest mapped status text as React state.
 *
 * Pass `null`/`undefined` to disable the subscription (for example
 * when the sender chose not to request a run_id). The hook handles
 * the full lifecycle: open the `EventSource` with the stored JWT,
 * forward events through {@link statusTextForAguiEvent}, close on
 * `RUN_FINISHED` / `RUN_ERROR` / unmount.
 *
 * Errors are deliberately silent — progress rendering is
 * best-effort, and the REST turn response (which carries the final
 * assistant reply) is the source of truth for the user-visible
 * conversation state.
 */
export function useAgUiProgress(runId: string | null | undefined): AgUiProgressState {
  const [statusText, setStatusText] = useState<string | null>(null);
  const [isActive, setIsActive] = useState(false);
  // Retain the EventSource across renders so cleanup can close it
  // even if the runId changes mid-turn.
  const sourceRef = useRef<EventSource | null>(null);

  useEffect(() => {
    if (!runId) {
      setStatusText(null);
      setIsActive(false);
      return;
    }

    let cancelled = false;
    setStatusText(null);
    setIsActive(true);

    (async () => {
      const token = await SecureStore.getItemAsync(JWT_KEY).catch(() => null);
      if (!token || cancelled) {
        // No auth token — we cannot subscribe. The turn will still
        // land via REST; the user just won't see intermediate
        // progress until the final reply renders.
        setIsActive(false);
        return;
      }

      const url = `${getApiUrl()}/api/agui/runs/${runId}/stream`;
      const source = new EventSource(url, {
        headers: { Authorization: `Bearer ${token}` },
        // The server closes the stream on `RunFinished`; we don't
        // want the SSE client to auto-reconnect after that, which
        // would re-open against a run id the registry already
        // dropped and log needless 404s.
        pollingInterval: 0,
      });
      sourceRef.current = source;

      source.addEventListener('message', (event) => {
        // react-native-sse types the data field as `string | null`;
        // guard against both.
        if (!event.data || cancelled) return;
        let parsed: AguiEventWire;
        try {
          parsed = JSON.parse(event.data) as AguiEventWire;
        } catch {
          return;
        }
        const text = statusTextForAguiEvent(parsed);
        if (text !== null) {
          setStatusText(text);
        }
        if (isTerminalAguiEvent(parsed)) {
          setIsActive(false);
          source.close();
        }
      });

      source.addEventListener('error', () => {
        // Connection failures are best-effort; surface as "no
        // progress available" and let the REST response deliver the
        // final reply.
        if (cancelled) return;
        setIsActive(false);
      });

      source.addEventListener('close', () => {
        if (cancelled) return;
        setIsActive(false);
      });
    })();

    return () => {
      cancelled = true;
      sourceRef.current?.close();
      sourceRef.current = null;
    };
  }, [runId]);

  return { statusText, isActive };
}
