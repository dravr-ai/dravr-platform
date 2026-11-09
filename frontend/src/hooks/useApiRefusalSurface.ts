// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Turns an authorization refusal into a toast carrying the server's own reason, app-wide
// ABOUTME: Stays silent while the transport is recovering a refusal by driving the athlete to sign in;
// ABOUTME: an expired provider session reads as one translated reconnect notice however many reads it failed

import { useEffect, useRef } from 'react';
import { useTranslation } from '@pierre/i18n';
import {
  classifyApiError,
  describeApiError,
  providerAuthRefusalKey,
  type ApiErrorTranslate,
} from '@pierre/ui-logic';
import { droveReauthentication } from '@pierre/api-client';
import { useToast } from '../components/ui';
import { CONNECTIONS_ROUTE } from '../constants/surfaceLayout';
import { refusalReporter } from '../services/queryClient';

/**
 * Take the athlete to the connections pane. The dashboard follows a hash
 * change itself, as it does for every in-app `location.hash` assignment.
 */
function openConnections(): void {
  window.location.hash = `#${CONNECTIONS_ROUTE}`;
}

/**
 * Install the app-wide surface for a refused API call.
 *
 * Scoped to the one kind of failure that has nowhere else to be said. A 403
 * carries a reason the athlete can act on — "Group coaching requires a
 * Professional or Enterprise plan", "Super-admin privileges required" — and
 * without this it reached them as `Request failed with status code 403`,
 * printed by whichever component happened to render `error.message`, on ten
 * admin tabs and on every refused group action. Every other kind is left to the
 * screen that made the call: ~15 components already render their own error
 * state, and a second global toast would double every one of them.
 *
 * A connected provider whose session expired refuses every read of it at once
 * — a dashboard drawing five Garmin routes gets five identical 403s — so that
 * refusal is keyed by its code and provider: one translated notice naming the
 * service, with the reconnect action, while it is on screen.
 *
 * Call it once, from inside both `AuthProvider` and `ToastProvider`.
 */
export function useApiRefusalSurface(): void {
  const { t } = useTranslation();
  const { addToast } = useToast();

  // Both are read through refs so the reporter is installed once. Neither is
  // stable: i18next returns a new `t` whenever the language resolves, and the
  // toast provider hands back a new `addToast` every render.
  const translate = useRef<ApiErrorTranslate>(t);
  translate.current = t;
  const toast = useRef(addToast);
  toast.current = addToast;

  useEffect(() => {
    refusalReporter.current = (error: unknown) => {
      // Asked of the refusal itself, not of the app's state. The response
      // interceptor decides which refusals recover by getting a new credential
      // — a 401, or the one 403 carrying RFC 6750's `insufficient_scope` — and
      // marks the rejection it hands on. Reading that mark means this surface
      // holds no second copy of the rule and cannot fall out of step with it.
      if (droveReauthentication(error)) {
        return;
      }
      if (classifyApiError(error).kind !== 'forbidden') {
        return;
      }
      const reconnectKey = providerAuthRefusalKey(error);
      toast.current({
        type: 'error',
        title: translate.current('common.error'),
        // A role refusal reads as the server's own sentence — `describeApiError`
        // prefers it for a 403 — and the generic refusal when it sent none. An
        // expired provider session reads as the translated reconnect sentence
        // naming the service, never the server's English or its backend id.
        message: describeApiError(error, { t: translate.current, fallbackKey: 'errors.forbidden' }),
        ...(reconnectKey === undefined
          ? {}
          : {
              dedupeKey: reconnectKey,
              // It carries the only way to reconnect, so it stays until the
              // athlete acts on it or closes it: a keyboard or screen-reader
              // athlete tabbing to the button must not lose it on a timer.
              duration: 0,
              action: { label: translate.current('providers.reconnect'), onClick: openConnections },
            }),
      });
    };
    return () => {
      refusalReporter.current = null;
    };
  }, []);
}
