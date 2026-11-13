// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Reads a login takeover's messages — its events, the step answer it ends with, the bridge to the phone app
// ABOUTME: One reading for web and mobile, so both word a takeover's end as they word the same answer to an HTTP step

import {
  SCIOTTE_LOGIN_PRESETS,
  SCIOTTE_TAKEOVER_ENDED,
  SCIOTTE_TAKEOVER_MAX_TEXT_BYTES,
  sciotteTargetForBackend,
} from '@pierre/shared-constants';
import type {
  SciotteLoginResponse,
  SciotteTakeoverBridgeMessage,
  SciotteTakeoverEvent,
  SciotteTakeoverOutcome,
  SciotteTarget,
} from '@pierre/shared-types';

/**
 * What a takeover's last event means to the window that ran it:
 *
 * - `answer`: a step answer, read as the same answer to `POST
 *   /api/providers/sciotte/login` is read (connected, failed, a code step…);
 * - `refusal`: a refusal, shaped as the HTTP client's error
 *   (`{ response: { status, data } }`), so `refusalReason` and
 *   `describeApiError` read it unchanged;
 * - `ended`: the takeover ended with nothing to say (its socket closed, or the
 *   scraper no longer held the sign-in) — the athlete connects again.
 */
export type TakeoverResult =
  | { kind: 'answer'; data: SciotteLoginResponse }
  | { kind: 'refusal'; error: { response: { status: number; data: unknown } } }
  | { kind: 'ended' };

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** The `details.reason` of an error body, when it names one. */
function bodyReason(body: unknown): string | undefined {
  if (!isRecord(body) || !isRecord(body.details)) return undefined;
  const reason = body.details.reason;
  return typeof reason === 'string' ? reason : undefined;
}

/** What `outcome` means to the window; see {@link TakeoverResult}. */
export function takeoverResult(outcome: SciotteTakeoverOutcome): TakeoverResult {
  const { status_code: status, body } = outcome;
  if (status < 400) {
    if (isRecord(body) && typeof body.status === 'string') {
      return { kind: 'answer', data: body as SciotteLoginResponse };
    }
    return { kind: 'ended' };
  }
  if (body === null || body === undefined || bodyReason(body) === SCIOTTE_TAKEOVER_ENDED) {
    return { kind: 'ended' };
  }
  return { kind: 'refusal', error: { response: { status, data: body } } };
}

/** `data` (a socket's text message) as a takeover event, or `null` for anything else. */
export function parseTakeoverEvent(data: string): SciotteTakeoverEvent | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(data);
  } catch {
    return null;
  }
  if (!isRecord(parsed)) return null;
  switch (parsed.type) {
    case 'status':
      if (parsed.state !== 'ready' || typeof parsed.width !== 'number' || typeof parsed.height !== 'number') {
        return null;
      }
      return { type: 'status', state: 'ready', width: parsed.width, height: parsed.height };
    case 'popup_refused':
      return { type: 'popup_refused' };
    case 'navigation_reset':
      return { type: 'navigation_reset' };
    case 'outcome':
      if (typeof parsed.status_code !== 'number') return null;
      return { type: 'outcome', status_code: parsed.status_code, body: parsed.body ?? null };
    default:
      return null;
  }
}

/** `data` (a WebView's posted string) as the takeover page's bridge message, or `null`. */
export function parseTakeoverBridgeMessage(data: string): SciotteTakeoverBridgeMessage | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(data);
  } catch {
    return null;
  }
  if (!isRecord(parsed)) return null;
  if (parsed.type === 'connected') {
    return typeof parsed.follow_on === 'string' && parsed.follow_on
      ? { type: 'connected', follow_on: parsed.follow_on }
      : { type: 'connected' };
  }
  if (parsed.type !== 'closed') return null;
  const outcome = parsed.outcome;
  if (isRecord(outcome) && outcome.type === 'outcome' && typeof outcome.status_code === 'number') {
    return {
      type: 'closed',
      outcome: { type: 'outcome', status_code: outcome.status_code, body: outcome.body ?? null },
    };
  }
  return { type: 'closed' };
}

/** The UTF-8 length of one code point. */
function utf8Length(codePoint: number): number {
  if (codePoint < 0x80) return 1;
  if (codePoint < 0x800) return 2;
  if (codePoint < 0x10000) return 3;
  return 4;
}

/**
 * `text` cut into pieces of at most `maxBytes` UTF-8 bytes each, never inside
 * a code point, in order. A takeover's text message carries at most
 * {@link SCIOTTE_TAKEOVER_MAX_TEXT_BYTES}, and one larger socket message is a
 * socket error that ends the sign-in, so a long paste goes as several.
 */
export function takeoverTextChunks(text: string, maxBytes: number = SCIOTTE_TAKEOVER_MAX_TEXT_BYTES): string[] {
  const chunks: string[] = [];
  let current = '';
  let bytes = 0;
  for (const char of text) {
    const length = utf8Length(char.codePointAt(0) ?? 0);
    if (bytes + length > maxBytes && current) {
      chunks.push(current);
      current = '';
      bytes = 0;
    }
    current += char;
    bytes += length;
  }
  if (current) chunks.push(current);
  return chunks;
}

/**
 * The backend provider a takeover token is for — its `tgt` claim — read for
 * display only (the page names the provider in its prompt); the server
 * verifies the token, never this. `null` for anything that is not a JWT
 * carrying one.
 */
export function takeoverTokenProvider(token: string, decodeBase64: (b64: string) => string): string | null {
  const payload = token.split('.')[1];
  if (!payload) return null;
  try {
    const b64 = payload.replace(/-/g, '+').replace(/_/g, '/');
    const padded = b64 + '='.repeat((4 - (b64.length % 4)) % 4);
    const claims: unknown = JSON.parse(decodeBase64(padded));
    return isRecord(claims) && typeof claims.tgt === 'string' && claims.tgt ? claims.tgt : null;
  } catch {
    return null;
  }
}

/**
 * The credential-login target a takeover's provider names, or `null`: the
 * backend name the login answers with (`sciotte_garmin`), or the target name
 * itself (`garmin`).
 */
export function takeoverTarget(provider: string): SciotteTarget | null {
  const byBackend = sciotteTargetForBackend(provider);
  if (byBackend) return byBackend;
  return Object.prototype.hasOwnProperty.call(SCIOTTE_LOGIN_PRESETS, provider) ? (provider as SciotteTarget) : null;
}
