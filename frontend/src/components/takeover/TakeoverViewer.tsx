// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The athlete's live view of a sign-in a provider's robot check parked — frames drawn on a canvas, their input sent back
// ABOUTME: Pointer on the canvas, keys and soft-keyboard text through a hidden field, a keyboard button for touch screens

import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from '@pierre/i18n';
import { takeoverResult, type TakeoverResult } from '@pierre/ui-logic';
import type { SciotteTakeoverInput, SciotteTakeoverOutcome } from '@pierre/shared-types';
import { Button, Input } from '../ui';
import { useTakeoverSocket, type TakeoverStage } from './useTakeoverSocket';
import { beforeInputMessages, forwardedKey, framePoint, textInputs } from './takeoverInput';

/** The most often a pointer move goes out: well inside the server's 60 messages a second. */
const MOVE_INTERVAL_MS = 50;
/** Scrolls inside this window go out as one. */
const WHEEL_INTERVAL_MS = 100;
/** The viewer's size settles this long before the page follows it. */
const RESIZE_SETTLE_MS = 250;

export interface TakeoverViewerProps {
  /** The takeover token, read from the takeover link's fragment. */
  token: string;
  /** The provider's name as the athlete knows it (Garmin, Strava…). */
  providerLabel: string;
  /**
   * Closing the window hands over to the provider's OAuth (carnet#1477): the
   * view says so, under the prompt.
   */
  oauthOffered?: boolean;
  /**
   * The takeover's answer: what the same login step would have answered,
   * with how to read it. Read here, in the viewer's own chunk, so the window
   * that opens it carries none of the takeover's code before first paint.
   */
  onOutcome: (outcome: SciotteTakeoverOutcome, result: TakeoverResult) => void;
  /** The takeover ended with no answer. */
  onEnded: () => void;
  /**
   * Sizes the view: the area the page's viewport follows. The default fits
   * the sign-in window's chrome on any screen, never below 320px.
   */
  viewClassName?: string;
}

/** The status line's key for each stage. */
const STAGE_KEYS: Record<TakeoverStage, string> = {
  connecting: 'shell.sciotteTakeoverConnecting',
  ready: 'shell.sciotteTakeoverPrompt',
  finishing: 'shell.sciotteTakeoverFinishing',
  ended: 'shell.sciotteTakeoverEnded',
};

/** One JPEG frame, decoded to something a canvas draws. */
async function decodeFrame(frame: Blob): Promise<CanvasImageSource & { width: number; height: number }> {
  if (typeof createImageBitmap === 'function') return createImageBitmap(frame);
  const url = URL.createObjectURL(frame);
  try {
    const image = new Image();
    image.src = url;
    await image.decode();
    return image;
  } finally {
    URL.revokeObjectURL(url);
  }
}

export default function TakeoverViewer({
  token,
  providerLabel,
  oauthOffered = false,
  onOutcome,
  onEnded,
  viewClassName = 'h-[max(320px,min(calc(100dvh_-_19rem),640px))]',
}: TakeoverViewerProps) {
  const { t } = useTranslation();
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const areaRef = useRef<HTMLDivElement>(null);
  const fieldRef = useRef<HTMLInputElement>(null);
  const [hasFrame, setHasFrame] = useState(false);
  // Frames decode asynchronously: only the newest one is drawn.
  const frameSeq = useRef(0);

  const onFrame = useCallback((frame: Blob) => {
    frameSeq.current += 1;
    const seq = frameSeq.current;
    decodeFrame(frame)
      .then((image) => {
        const canvas = canvasRef.current;
        if (!canvas || seq !== frameSeq.current) return;
        if (canvas.width !== image.width || canvas.height !== image.height) {
          canvas.width = image.width;
          canvas.height = image.height;
        }
        canvas.getContext('2d')?.drawImage(image, 0, 0);
        if ('close' in image && typeof image.close === 'function') image.close();
        setHasFrame(true);
      })
      .catch(() => undefined);
  }, []);

  const readOutcome = useCallback(
    (outcome: SciotteTakeoverOutcome) => onOutcome(outcome, takeoverResult(outcome)),
    [onOutcome],
  );
  const { stage, viewport, notice, clearNotice, send } = useTakeoverSocket(token, {
    onFrame,
    onOutcome: readOutcome,
    onEnded,
  });
  const live = stage === 'ready';

  const sendAll = useCallback((inputs: SciotteTakeoverInput[]) => inputs.forEach(send), [send]);

  // The frame's coordinate space: the drawn frame, else the viewport the server named.
  const frameSize = useCallback((): { width: number; height: number } | null => {
    const canvas = canvasRef.current;
    if (canvas && hasFrame) return { width: canvas.width, height: canvas.height };
    return viewport;
  }, [hasFrame, viewport]);

  const pointAt = useCallback(
    (event: { clientX: number; clientY: number }) => {
      const canvas = canvasRef.current;
      const frame = frameSize();
      if (!canvas || !frame) return null;
      return framePoint(event.clientX, event.clientY, canvas.getBoundingClientRect(), frame);
    },
    [frameSize],
  );

  // Moves are coalesced to the latest, at most one per MOVE_INTERVAL_MS;
  // a press or a release first sends any move still waiting, in order.
  const pendingMove = useRef<{ x: number; y: number } | null>(null);
  const moveTimer = useRef<number | null>(null);
  const flushMove = useCallback(() => {
    if (moveTimer.current !== null) {
      window.clearTimeout(moveTimer.current);
      moveTimer.current = null;
    }
    const point = pendingMove.current;
    pendingMove.current = null;
    if (point) send({ type: 'pointer', action: 'move', ...point });
  }, [send]);
  const pressed = useRef(false);

  const onPointerDown = (event: React.PointerEvent<HTMLCanvasElement>) => {
    event.preventDefault();
    clearNotice();
    const point = pointAt(event);
    if (!point) return;
    flushMove();
    pressed.current = true;
    event.currentTarget.setPointerCapture?.(event.pointerId);
    send({ type: 'pointer', action: 'down', ...point });
    // A mouse brings the keyboard with it; a touch screen's keyboard waits
    // for the keyboard button, so a tap does not raise it over the page.
    if (event.pointerType === 'mouse') fieldRef.current?.focus({ preventScroll: true });
  };
  const onPointerUp = (event: React.PointerEvent<HTMLCanvasElement>) => {
    event.preventDefault();
    if (!pressed.current) return;
    pressed.current = false;
    const point = pointAt(event);
    if (!point) return;
    flushMove();
    send({ type: 'pointer', action: 'up', ...point });
  };
  const onPointerMove = (event: React.PointerEvent<HTMLCanvasElement>) => {
    // A mouse hovers; a finger only moves while it drags (a slider check).
    if (event.pointerType !== 'mouse' && !pressed.current) return;
    const point = pointAt(event);
    if (!point) return;
    pendingMove.current = point;
    if (moveTimer.current === null) moveTimer.current = window.setTimeout(flushMove, MOVE_INTERVAL_MS);
  };

  // Wheel: a native, non-passive listener (React's is passive, so it could
  // not keep the page behind from scrolling), summed per WHEEL_INTERVAL_MS.
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    let pending: { x: number; y: number; dx: number; dy: number } | null = null;
    let timer: number | null = null;
    const flush = () => {
      timer = null;
      if (pending) send({ type: 'wheel', ...pending });
      pending = null;
    };
    const onWheel = (event: WheelEvent) => {
      event.preventDefault();
      const point = pointAt(event);
      if (!point) return;
      pending = { ...point, dx: (pending?.dx ?? 0) + event.deltaX, dy: (pending?.dy ?? 0) + event.deltaY };
      if (timer === null) timer = window.setTimeout(flush, WHEEL_INTERVAL_MS);
    };
    canvas.addEventListener('wheel', onWheel, { passive: false });
    return () => {
      canvas.removeEventListener('wheel', onWheel);
      if (timer !== null) window.clearTimeout(timer);
    };
  }, [pointAt, send]);

  // Text: native beforeinput, paste and compositionend on the hidden field.
  // React's onBeforeInput is a polyfill that carries no inputType.
  useEffect(() => {
    const field = fieldRef.current;
    if (!field) return;
    const onBeforeInput = (event: InputEvent) => {
      const messages = beforeInputMessages({
        inputType: event.inputType,
        data: event.data,
        isComposing: event.isComposing,
        pasted: event.dataTransfer?.getData('text/plain') ?? null,
      });
      if (messages === null) return;
      event.preventDefault();
      sendAll(messages);
    };
    const onPaste = (event: ClipboardEvent) => {
      const text = event.clipboardData?.getData('text/plain');
      if (!text) return;
      event.preventDefault();
      sendAll(textInputs(text));
    };
    // Composed text (an IME, autocorrect) lands in the field: forward it
    // once composed, then empty the field.
    const onCompositionEnd = () => {
      if (field.value) sendAll(textInputs(field.value));
      field.value = '';
    };
    field.addEventListener('beforeinput', onBeforeInput);
    field.addEventListener('paste', onPaste);
    field.addEventListener('compositionend', onCompositionEnd);
    return () => {
      field.removeEventListener('beforeinput', onBeforeInput);
      field.removeEventListener('paste', onPaste);
      field.removeEventListener('compositionend', onCompositionEnd);
    };
  }, [sendAll]);

  // Keys the page needs as keys. Stopped here, so the dialog around the view
  // neither closes on the page's Escape nor moves focus on its Tab.
  const onKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    const key = forwardedKey(event);
    if (!key) return;
    event.preventDefault();
    event.stopPropagation();
    send({ type: 'key', action: 'down', key });
  };
  const onKeyUp = (event: React.KeyboardEvent<HTMLInputElement>) => {
    const key = forwardedKey(event);
    if (!key) return;
    event.preventDefault();
    event.stopPropagation();
    send({ type: 'key', action: 'up', key });
  };

  // The page's viewport follows the view's size, once it streams.
  const lastSize = useRef<string | null>(null);
  useEffect(() => {
    const area = areaRef.current;
    if (!area || !live) return;
    let timer: number | null = null;
    const report = () => {
      timer = null;
      const width = Math.round(area.clientWidth);
      const height = Math.round(area.clientHeight);
      if (width <= 0 || height <= 0) return;
      const key = `${width}x${height}`;
      if (key === lastSize.current) return;
      lastSize.current = key;
      send({ type: 'resize', width, height });
    };
    report();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(() => {
      if (timer !== null) window.clearTimeout(timer);
      timer = window.setTimeout(report, RESIZE_SETTLE_MS);
    });
    observer.observe(area);
    return () => {
      observer.disconnect();
      if (timer !== null) window.clearTimeout(timer);
    };
  }, [live, send]);

  useEffect(
    () => () => {
      if (moveTimer.current !== null) window.clearTimeout(moveTimer.current);
    },
    [],
  );

  const noticeKey =
    notice === 'popup_refused'
      ? 'shell.sciotteTakeoverPopupRefused'
      : notice === 'navigation_reset'
        ? 'shell.sciotteTakeoverPageReset'
        : null;

  return (
    <div className="flex flex-col gap-3" data-testid="takeover-viewer" data-stage={stage}>
      {/* The words and the keyboard button above the view, so neither ever
          falls below the fold of a short screen. */}
      <div className="flex items-start gap-3">
        <div className="flex-1 min-w-0 space-y-1">
          <p role="status" className="text-sm font-medium text-on-surface" data-testid="takeover-status">
            {t(STAGE_KEYS[stage], { provider: providerLabel })}
          </p>
          {noticeKey && live && (
            <p
              role="status"
              className="text-sm text-on-warning-container rounded-lg bg-warning/10 border border-warning/20 px-3 py-2"
              data-testid="takeover-notice"
            >
              {t(noticeKey, { provider: providerLabel })}
            </p>
          )}
          {oauthOffered && stage !== 'ended' && (
            <p className="text-xs text-on-surface-variant" data-testid="takeover-oauth-offer">
              {t('shell.sciotteTakeoverOAuthOffer', { provider: providerLabel })}
            </p>
          )}
        </div>
        <Button
          variant="secondary"
          size="sm"
          className="flex-shrink-0"
          onClick={() => fieldRef.current?.focus({ preventScroll: true })}
          disabled={!live}
          data-testid="takeover-keyboard-button"
        >
          {t('shell.sciotteTakeoverKeyboard')}
        </Button>
      </div>

      <div
        ref={areaRef}
        className={`relative w-full overflow-hidden rounded-xl border ghost-border bg-surface-container-lowest flex items-center justify-center ${viewClassName}`}
      >
        <canvas
          ref={canvasRef}
          role="img"
          aria-label={t('shell.sciotteTakeoverViewAria', { provider: providerLabel })}
          data-testid="takeover-canvas"
          className={`block max-w-full max-h-full touch-none select-none ${hasFrame ? '' : 'invisible'}`}
          onPointerDown={onPointerDown}
          onPointerUp={onPointerUp}
          onPointerCancel={onPointerUp}
          onPointerMove={onPointerMove}
          onContextMenu={(event) => event.preventDefault()}
        />
        {!hasFrame && stage !== 'ended' && (
          <div className="absolute inset-0 flex items-center justify-center" aria-hidden="true">
            <div className="pierre-spinner w-12 h-12" />
          </div>
        )}
        {/* The keyboard's way in, never shown: every key and every typed
            character reaches the page through it. 16px text (the large size),
            so iOS does not zoom the page when it takes focus. */}
        <div className="absolute left-0 top-0 w-px h-px overflow-hidden opacity-0 pointer-events-none">
          <Input
            ref={fieldRef}
            type="text"
            size="lg"
            aria-label={t('shell.sciotteTakeoverKeyboard')}
            data-testid="takeover-keyboard-field"
            autoCapitalize="off"
            autoComplete="off"
            autoCorrect="off"
            spellCheck={false}
            enterKeyHint="go"
            onKeyDown={onKeyDown}
            onKeyUp={onKeyUp}
            disabled={!live}
          />
        </div>
      </div>
    </div>
  );
}
