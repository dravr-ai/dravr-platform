// ABOUTME: Web hook that subscribes to the AG-UI SSE stream and surfaces per-event progress status text
// ABOUTME: Uses fetch + ReadableStream (browser EventSource cannot set the Bearer header the stream requires)

import { useEffect, useRef, useState } from 'react';
import {
  statusTextForAguiEvent,
  isTerminalAguiEvent,
  type AguiEventWire,
} from '@pierre/chat-utils';

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
 * the full lifecycle: open the stream with the supplied JWT, forward
 * events through {@link statusTextForAguiEvent}, close on
 * `RUN_FINISHED` / `RUN_ERROR` / unmount.
 *
 * Unlike the mobile sibling (`react-native-sse`), the browser's
 * native `EventSource` cannot attach an `Authorization` header, and
 * the AG-UI endpoint is Bearer-only. So we consume the SSE stream the
 * same way `ChatTab` consumes the chat POST — `fetch()` returning a
 * `ReadableStream` we frame-parse ourselves.
 *
 * Errors are deliberately silent — progress rendering is best-effort,
 * and the chat SSE response (which carries the streamed assistant
 * reply) is the source of truth for the user-visible conversation.
 */
export function useAgUiProgress(
  runId: string | null | undefined,
  token: string | null | undefined,
): AgUiProgressState {
  const [statusText, setStatusText] = useState<string | null>(null);
  const [isActive, setIsActive] = useState(false);
  // Retain the AbortController across renders so cleanup can cancel
  // the in-flight fetch even if the runId changes mid-turn.
  const controllerRef = useRef<AbortController | null>(null);

  useEffect(() => {
    if (!runId || !token) {
      setStatusText(null);
      setIsActive(false);
      return;
    }

    let cancelled = false;
    const controller = new AbortController();
    controllerRef.current = controller;
    setStatusText(null);
    setIsActive(true);

    const handleEvent = (parsed: AguiEventWire): boolean => {
      const text = statusTextForAguiEvent(parsed);
      if (text !== null && !cancelled) {
        setStatusText(text);
      }
      return isTerminalAguiEvent(parsed);
    };

    (async () => {
      try {
        const response = await fetch(`/api/agui/runs/${runId}/stream`, {
          method: 'GET',
          headers: {
            Accept: 'text/event-stream',
            Authorization: `Bearer ${token}`,
          },
          signal: controller.signal,
        });

        if (!response.ok || !response.body) {
          // The chat SSE turn still lands the final reply; the user
          // just won't see intermediate progress.
          if (!cancelled) setIsActive(false);
          return;
        }

        const reader = response.body.getReader();
        const decoder = new TextDecoder();
        let buffer = '';

        // SSE frames are separated by a blank line. Each frame carries
        // an `event:` name (`agui`, `connection`, `keepalive`) and one
        // or more `data:` lines. Only `agui` frames carry the JSON
        // event payload we map to status text.
        while (!cancelled) {
          const { done, value } = await reader.read();
          if (done) break;
          buffer += decoder.decode(value, { stream: true });

          let sep: number;
          while ((sep = buffer.indexOf('\n\n')) !== -1) {
            const chunk = buffer.slice(0, sep);
            buffer = buffer.slice(sep + 2);

            let eventName = 'message';
            const dataLines: string[] = [];
            for (const line of chunk.split('\n')) {
              if (line.startsWith('event:')) {
                eventName = line.slice(line[6] === ' ' ? 7 : 6).trim();
              } else if (line.startsWith('data:')) {
                dataLines.push(line.slice(line[5] === ' ' ? 6 : 5));
              }
            }

            if (eventName !== 'agui' || dataLines.length === 0) continue;

            let parsed: AguiEventWire;
            try {
              parsed = JSON.parse(dataLines.join('\n')) as AguiEventWire;
            } catch {
              continue;
            }

            const terminal = handleEvent(parsed);
            if (terminal) {
              if (!cancelled) setIsActive(false);
              controller.abort();
              return;
            }
          }
        }

        if (!cancelled) setIsActive(false);
      } catch {
        // AbortError (cleanup / terminal) and network failures alike:
        // progress is best-effort, so collapse the strip silently.
        if (!cancelled) setIsActive(false);
      }
    })();

    return () => {
      cancelled = true;
      controllerRef.current?.abort();
      controllerRef.current = null;
    };
  }, [runId, token]);

  return { statusText, isActive };
}
