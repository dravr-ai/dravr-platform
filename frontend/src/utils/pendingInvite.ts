// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The group invite an athlete opened, held in localStorage until its /group join turn is answered
// ABOUTME: Shared by every tab of the browser, so the email-confirmation tab and a reload after sign-in still join

/** The `localStorage` key holding the invite opened at `/groups/join/:code`. */
export const PENDING_INVITE_KEY = 'dravr.pending_invite_code';

/**
 * How long a stored invite still stands for the athlete who opened it: long
 * enough to confirm an email and sign in, short enough that a shared browser
 * does not join the next account to sign in on it.
 */
export const PENDING_INVITE_TTL_MS = 24 * 60 * 60 * 1000;

/** What [`PENDING_INVITE_KEY`] holds. */
interface StoredInvite {
  code: string;
  expiresAt: number;
}

function isStoredInvite(value: unknown): value is StoredInvite {
  if (typeof value !== 'object' || value === null) return false;
  const { code, expiresAt } = value as Record<string, unknown>;
  return typeof code === 'string' && code.length > 0 && typeof expiresAt === 'number';
}

/** Forget the stored invite. Storage refused: there is nothing to forget. */
export function clearPendingInvite(): void {
  try {
    window.localStorage.removeItem(PENDING_INVITE_KEY);
  } catch {
    // Storage refused (a private window): nothing was stored.
  }
}

/**
 * The invite this browser opened and has not yet joined, or null when there is
 * none, it has expired or it cannot be read. An expired or unreadable entry is
 * removed on the way.
 *
 * An invite stored by the earlier, per-tab version (a bare code in
 * `sessionStorage`) is moved to `localStorage`, so a tab open across the
 * release still joins.
 */
export function readPendingInvite(now: number = Date.now()): string | null {
  let raw: string | null;
  try {
    raw = window.localStorage.getItem(PENDING_INVITE_KEY);
  } catch {
    return null;
  }
  if (raw === null) return adoptTabInvite(now);
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    parsed = null;
  }
  if (!isStoredInvite(parsed) || parsed.expiresAt <= now) {
    clearPendingInvite();
    return null;
  }
  return parsed.code;
}

/** Hold `code` for every tab of this browser until its join is answered. */
export function writePendingInvite(code: string, now: number = Date.now()): void {
  const stored: StoredInvite = { code, expiresAt: now + PENDING_INVITE_TTL_MS };
  try {
    window.localStorage.setItem(PENDING_INVITE_KEY, JSON.stringify(stored));
  } catch {
    // Storage refused: the invite lasts as long as the page.
  }
}

/** Move a per-tab invite left in `sessionStorage` into `localStorage`. */
function adoptTabInvite(now: number): string | null {
  try {
    const code = window.sessionStorage.getItem(PENDING_INVITE_KEY);
    if (!code) return null;
    window.sessionStorage.removeItem(PENDING_INVITE_KEY);
    writePendingInvite(code, now);
    return code;
  } catch {
    return null;
  }
}

/**
 * Call `onCleared` when another tab forgets the stored invite — its join was
 * answered there — so this tab does not send the same join again. Answers the
 * unsubscribe.
 */
export function onPendingInviteClearedElsewhere(onCleared: () => void): () => void {
  const listener = (event: StorageEvent) => {
    // `key === null` is a `localStorage.clear()` from another tab.
    if (event.key !== null && event.key !== PENDING_INVITE_KEY) return;
    if (event.newValue === null) onCleared();
  };
  window.addEventListener('storage', listener);
  return () => window.removeEventListener('storage', listener);
}
