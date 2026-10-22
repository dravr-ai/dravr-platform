// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The phone's reading of a signup code refusal, shared by the register form and the pending screen
// ABOUTME: An invalid code (details.reason) gets the translated sentence; other refusals keep their own

import {
  classifyApiError,
  describeApiError,
  isInvalidSignupCode,
  type ApiErrorTranslate,
} from '@pierre/ui-logic';

/**
 * The sentence a failed code redemption shows under its field. The other
 * refusals (already active, a code already used, sign-up paused, too many
 * attempts) carry a sentence of their own, which is kept.
 */
export function describeSignupCodeError(error: unknown, t: ApiErrorTranslate): string {
  if (isInvalidSignupCode(error)) {
    return t('auth.signupCodeInvalid');
  }
  const { status, detail } = classifyApiError(error);
  if (status === 429 && detail) {
    return detail;
  }
  return describeApiError(error, { t, fallbackKey: 'errors.unknown' });
}

/** The code as sent: trimmed, or `undefined` when nothing was entered. */
export function signupCodeOrUndefined(raw: string): string | undefined {
  const code = raw.trim();
  return code === '' ? undefined : code;
}
