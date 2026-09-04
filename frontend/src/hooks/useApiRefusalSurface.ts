// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Turns an authorization refusal into a toast carrying the server's own reason, app-wide
// ABOUTME: Stays silent while the transport is recovering a refusal by driving the athlete to sign in

import { useEffect, useRef } from 'react';
import { useTranslation } from '@pierre/i18n';
import { classifyApiError, describeApiError, type ApiErrorTranslate } from '@pierre/ui-logic';
import { droveReauthentication } from '@pierre/api-client';
import { useErrorToast } from '../components/ui';
import { refusalReporter } from '../services/queryClient';

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
 * Call it once, from inside both `AuthProvider` and `ToastProvider`.
 */
export function useApiRefusalSurface(): void {
  const { t } = useTranslation();
  const showError = useErrorToast();

  // Both are read through refs so the reporter is installed once. Neither is
  // stable: i18next returns a new `t` whenever the language resolves, and
  // `useErrorToast` returns a new function every render.
  const translate = useRef<ApiErrorTranslate>(t);
  translate.current = t;
  const toast = useRef(showError);
  toast.current = showError;

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
      toast.current(
        translate.current('common.error'),
        // The server's own sentence when it sent one — `describeApiError`
        // prefers it for a 403 — and the generic refusal when it did not.
        describeApiError(error, { t: translate.current, fallbackKey: 'errors.forbidden' }),
      );
    };
    return () => {
      refusalReporter.current = null;
    };
  }, []);
}
