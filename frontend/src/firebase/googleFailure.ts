// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The banner text for a Google sign-in that failed, in the athlete's locale
// ABOUTME: Imports no Firebase SDK, so the forms load it statically; the SDK stays lazy

import { classifyApiError } from '@pierre/ui-logic';

/**
 * What to tell the athlete when a Google sign-in fails.
 *
 * Firebase reports a dead network as `auth/network-request-failed` rather than
 * as an absent HTTP response, so that code is folded in before the shared
 * classifier sees the error. Anything unclassified gets the generic Google
 * message, so raw SDK strings ("A network AuthError…") never reach athletes.
 */
export function describeGoogleFailure(
  err: unknown,
  online: boolean,
  t: (key: string) => string,
): string {
  const code = (err as { code?: string }).code;
  if (code === 'auth/network-request-failed') {
    return online ? t('errors.network') : t('errors.offline');
  }
  const { kind } = classifyApiError(err, { online });
  if (kind === 'offline') {
    return t('errors.offline');
  }
  if (kind === 'network' || kind === 'timeout') {
    return t('errors.network');
  }
  if (kind === 'server') {
    return t('errors.serverError');
  }
  return t('auth.googleSignInFailed');
}
