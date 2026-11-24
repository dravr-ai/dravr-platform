// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Words a sign-in the provider turned down: its own reason, quoted, and what the athlete does next
// ABOUTME: Shared by the web and mobile sign-in windows so both say the same thing in the athlete's language

/**
 * The failure a sign-in window shows when the provider turned the sign-in
 * down (`status: failed`).
 *
 * The provider's own words come quoted, so the athlete sees why: an unknown
 * email ("Couldn't find this account") reads differently from a wrong
 * password. The advice after them names the way on. A window that offers
 * Google and Apple (`offersMethodChoice`) points an email sign-in at them,
 * since an account opened with either has no password at the provider. The
 * card's OAuth, where it has one, is the window's own button.
 */
export function signInTurnedDown(
  t: (key: string, params?: Record<string, string>) => string,
  provider: string,
  reason: string | null | undefined,
  offersMethodChoice: boolean,
): string {
  const advice = offersMethodChoice
    ? t('shell.sciotteTurnedDownAdviceMethods', { provider })
    : t('shell.sciotteTurnedDownAdvice');
  const quoted = reason?.trim();
  return quoted
    ? `${t('shell.sciotteTurnedDownReason', { provider, reason: quoted })} ${advice}`
    : advice;
}
