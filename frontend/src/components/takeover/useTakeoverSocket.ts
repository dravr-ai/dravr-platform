// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The one WebSocket of a login takeover — frames and events in, the athlete's input out, closed only on unmount
// ABOUTME: Closing it ends the sign-in on the server, so it is dialled once per token and never redialled

import { useCallback, useEffect, useRef, useState } from 'react';
import { parseTakeoverEvent } from '@pierre/ui-logic';
import type { SciotteTakeoverInput, SciotteTakeoverOutcome } from '@pierre/shared-types';
import { oauthApi } from '../../services/api';

/**
 * Where a takeover stands: dialling (`connecting`), streaming and taking
 * input (`ready`), its answer arrived and the window is handing over
 * (`finishing`), or its socket closed with no answer (`ended`).
 */
export type TakeoverStage = 'connecting' | 'ready' | 'finishing' | 'ended';

/** A passing message the page caused: a refused popup, or a navigation sent back. */
export type TakeoverNotice = 'popup_refused' | 'navigation_reset';

/** What the socket tells the window that holds it. */
export interface TakeoverSocketHandlers {
  /** One JPEG frame of the page, as the socket delivered it. */
  onFrame: (frame: Blob) => void;
  /** The takeover's last event: the step answer, as an HTTP step would have answered. */
  onOutcome: (outcome: SciotteTakeoverOutcome) => void;
  /** The socket closed with no answer: the sign-in is over. */
  onEnded: () => void;
}

export interface TakeoverSocket {
  stage: TakeoverStage;
  /** The page's viewport, which is the frames' size, once the server named it. */
  viewport: { width: number; height: number } | null;
  /** The latest passing message, until {@link TakeoverSocket.clearNotice}. */
  notice: TakeoverNotice | null;
  clearNotice: () => void;
  /** Send one input message; dropped unless the socket is open and streaming. */
  send: (input: SciotteTakeoverInput) => void;
}

/**
 * The takeover socket for `token`.
 *
 * Any close of it, the browser's included, ends the sign-in on the server and
 * a second socket is refused, so this dials once per token and closes only
 * when the window unmounts. Handlers are read through a ref, so a re-render
 * with new callbacks neither closes nor redials. The dial waits one task: a
 * development StrictMode mount that is torn down at once never opens a
 * socket, which would have cancelled the sign-in before the real mount ran.
 */
export function useTakeoverSocket(token: string, handlers: TakeoverSocketHandlers): TakeoverSocket {
  const [stage, setStage] = useState<TakeoverStage>('connecting');
  const [viewport, setViewport] = useState<{ width: number; height: number } | null>(null);
  const [notice, setNotice] = useState<TakeoverNotice | null>(null);
  const socketRef = useRef<WebSocket | null>(null);
  const handlersRef = useRef(handlers);
  // Whether the server has said the page streams: input before that is dropped.
  const readyRef = useRef(false);

  useEffect(() => {
    handlersRef.current = handlers;
  });

  useEffect(() => {
    setStage('connecting');
    setViewport(null);
    setNotice(null);
    readyRef.current = false;
    let socket: WebSocket | null = null;
    let answered = false;

    const dial = window.setTimeout(() => {
      const url = oauthApi.sciotteTakeoverSocketUrl(token, window.location.origin);
      if (!url) {
        setStage('ended');
        handlersRef.current.onEnded();
        return;
      }
      socket = new WebSocket(url);
      socket.binaryType = 'blob';
      socketRef.current = socket;
      socket.onmessage = (event: MessageEvent<unknown>) => {
        if (event.data instanceof Blob) {
          handlersRef.current.onFrame(event.data);
          return;
        }
        if (typeof event.data !== 'string') return;
        const parsed = parseTakeoverEvent(event.data);
        if (!parsed) return;
        switch (parsed.type) {
          case 'status':
            readyRef.current = true;
            setViewport({ width: parsed.width, height: parsed.height });
            setStage('ready');
            break;
          case 'popup_refused':
          case 'navigation_reset':
            setNotice(parsed.type);
            break;
          case 'outcome':
            answered = true;
            readyRef.current = false;
            setStage('finishing');
            handlersRef.current.onOutcome(parsed);
            break;
        }
      };
      socket.onclose = () => {
        socketRef.current = null;
        readyRef.current = false;
        if (answered) return;
        setStage('ended');
        handlersRef.current.onEnded();
      };
    }, 0);

    return () => {
      window.clearTimeout(dial);
      if (socket) {
        socket.onmessage = null;
        socket.onclose = null;
        if (socket.readyState === WebSocket.CONNECTING || socket.readyState === WebSocket.OPEN) {
          socket.close(1000);
        }
      }
      socketRef.current = null;
      readyRef.current = false;
    };
  }, [token]);

  const send = useCallback((input: SciotteTakeoverInput) => {
    const socket = socketRef.current;
    if (!socket || socket.readyState !== WebSocket.OPEN || !readyRef.current) return;
    socket.send(JSON.stringify(input));
  }, []);

  const clearNotice = useCallback(() => setNotice(null), []);

  return { stage, viewport, notice, clearNotice, send };
}
