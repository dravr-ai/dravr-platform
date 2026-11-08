// ABOUTME: The admin console's fixed locale: dates and failure sentences on admin-only screens read English in any session
// ABOUTME: Athlete screens read the viewer's locale; these screens never do, so nothing on them is half translated

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { classifyApiError, prefersServerDetail, type ApiErrorKind } from '@pierre/ui-logic';

/** What admin-only screens format dates in. */
export const ADMIN_LOCALE = 'en';

/**
 * The transport kinds in English. `describeApiError` reads these from the
 * catalogue, but the web bundle embeds only the default locale and loads the
 * others on demand, so a French session cannot read the English sentence
 * synchronously.
 */
const ENGLISH_SENTENCE: Partial<Record<ApiErrorKind, string>> = {
  offline: "You're offline. Check your connection and try again.",
  network: 'Network error. Check your connection.',
  timeout: 'The request timed out. Try again.',
  unauthorized: 'Your session expired. Sign in again.',
  server: 'Server error. Try again a bit later.',
};

/**
 * A failed call in one English line, under the same rule as `describeApiError`:
 * a transport failure is named, the server's own detail is shown only for the
 * kinds where it is safe, and anything else says `fallback`.
 */
export function describeAdminError(err: unknown, fallback = 'Something went wrong'): string {
  const { kind, detail } = classifyApiError(err);
  const sentence = ENGLISH_SENTENCE[kind];
  if (sentence !== undefined) {
    return sentence;
  }
  return detail && prefersServerDetail(kind) ? detail : fallback;
}
