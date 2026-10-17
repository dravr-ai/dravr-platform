// ABOUTME: Cloudflare Turnstile bot check, rendered explicitly into a container the form owns
// ABOUTME: Loads the Turnstile script once per page, reports the token, and resets on demand
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { forwardRef, useEffect, useImperativeHandle, useRef } from 'react';

/** The explicit-render script: nothing renders until `turnstile.render` is called. */
export const TURNSTILE_SCRIPT_URL = 'https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit';

/** The subset of Turnstile's render options this app passes. */
export interface TurnstileRenderOptions {
  sitekey: string;
  callback: (token: string) => void;
  'expired-callback': () => void;
  'error-callback': () => void;
  theme?: 'auto' | 'light' | 'dark';
}

/** The global Cloudflare's script installs. */
export interface TurnstileApi {
  render: (container: HTMLElement, options: TurnstileRenderOptions) => string;
  reset: (widgetId?: string) => void;
  remove: (widgetId?: string) => void;
}

declare global {
  interface Window {
    turnstile?: TurnstileApi;
  }
}

let scriptLoad: Promise<TurnstileApi> | null = null;

/**
 * Resolve once `window.turnstile` exists, injecting the script the first time.
 *
 * One script per page however many widgets mount: a second tag would load the
 * API twice. A failed load is forgotten so the next mount tries again.
 */
function loadTurnstile(): Promise<TurnstileApi> {
  if (window.turnstile) {
    return Promise.resolve(window.turnstile);
  }
  if (scriptLoad) {
    return scriptLoad;
  }
  scriptLoad = new Promise<TurnstileApi>((resolve, reject) => {
    const script = document.createElement('script');
    script.src = TURNSTILE_SCRIPT_URL;
    script.async = true;
    script.defer = true;
    script.onload = () => {
      if (window.turnstile) {
        resolve(window.turnstile);
      } else {
        reject(new Error('turnstile_global_missing'));
      }
    };
    script.onerror = () => reject(new Error('turnstile_script_load_failed'));
    document.head.appendChild(script);
  }).catch((err: unknown) => {
    scriptLoad = null;
    throw err;
  });
  return scriptLoad;
}

export interface TurnstileWidgetHandle {
  /** Discard the current token and ask for a new challenge: a token is single-use. */
  reset: () => void;
}

export interface TurnstileWidgetProps {
  siteKey: string;
  /** The token once the visitor passes, `null` once it expires or the widget errors. */
  onToken: (token: string | null) => void;
  /** The script could not be loaded (offline, or blocked by an extension). */
  onLoadError?: () => void;
  className?: string;
}

export const TurnstileWidget = forwardRef<TurnstileWidgetHandle, TurnstileWidgetProps>(
  function TurnstileWidget({ siteKey, onToken, onLoadError, className }, ref) {
    const containerRef = useRef<HTMLDivElement>(null);
    const widgetIdRef = useRef<string | null>(null);
    // Read through refs so a parent re-render with new closures does not
    // tear the widget down and make the visitor solve it again.
    const onTokenRef = useRef(onToken);
    const onLoadErrorRef = useRef(onLoadError);
    useEffect(() => {
      onTokenRef.current = onToken;
      onLoadErrorRef.current = onLoadError;
    });

    useImperativeHandle(ref, () => ({
      reset: () => {
        onTokenRef.current(null);
        if (widgetIdRef.current !== null) {
          window.turnstile?.reset(widgetIdRef.current);
        }
      },
    }), []);

    useEffect(() => {
      let cancelled = false;
      loadTurnstile()
        .then((turnstile) => {
          const container = containerRef.current;
          if (cancelled || !container) return;
          widgetIdRef.current = turnstile.render(container, {
            sitekey: siteKey,
            callback: (token) => onTokenRef.current(token),
            'expired-callback': () => onTokenRef.current(null),
            'error-callback': () => onTokenRef.current(null),
            theme: 'auto',
          });
        })
        .catch(() => {
          if (!cancelled) onLoadErrorRef.current?.();
        });
      return () => {
        cancelled = true;
        if (widgetIdRef.current !== null) {
          window.turnstile?.remove(widgetIdRef.current);
          widgetIdRef.current = null;
        }
      };
    }, [siteKey]);

    return <div ref={containerRef} className={className} data-testid="turnstile-widget" />;
  },
);
