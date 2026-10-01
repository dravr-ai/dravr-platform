// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Words a failed chat turn from the shared catalogue, keyed on the server's stable error code
// ABOUTME: Both clients render this one sentence; the server's English text never reaches the thread

import { TurnFailedError, TurnIdleAbortedError } from '@pierre/api-client';
import {
  API_ERROR_KEYS,
  classifyApiError,
  describeQuotaRefusal,
  quotaKey,
  type ApiErrorTranslate,
} from './apiError';

/**
 * The sentence each server error code reads as when it ends a turn.
 *
 * Keyed on `ErrorCode`'s wire names — the `code` of a `failed` frame and of a
 * refused turn's JSON body alike. A turn is the athlete asking their agent
 * something, so an upstream outage, a shed request and a throttled provider
 * all read as the same thing to them: Dravr is briefly unavailable, try again.
 */
const TURN_CODE_KEYS: Readonly<Record<string, string>> = {
  ResourceUnavailable: 'errors.generic',
  ExternalServiceUnavailable: 'errors.generic',
  ExternalServiceError: 'errors.generic',
  ExternalRateLimited: 'errors.generic',
  RateLimitExceeded: 'errors.generic',
  AuthRequired: 'errors.unauthorized',
  AuthInvalid: 'errors.unauthorized',
  AuthExpired: 'errors.unauthorized',
  AuthMalformed: 'errors.unauthorized',
  PermissionDenied: 'errors.forbidden',
  ResourceNotFound: 'errors.notFound',
  InvalidInput: 'errors.validation',
  MissingRequiredField: 'errors.validation',
  InvalidFormat: 'errors.validation',
  ValueOutOfRange: 'errors.validation',
};

/** What a turn that ended with no usable reason reads as. */
const TURN_UNWORDED_KEY = 'errors.serverError';

/** The counts a quota refusal names, when it names them. */
interface QuotaDetails {
  limit_type?: unknown;
  current?: unknown;
  limit?: unknown;
  /** Why the cap applies here — `conversation_archived` for an archived thread. */
  reason?: unknown;
}

function asQuota(details: unknown): QuotaDetails | null {
  return details !== null && typeof details === 'object' ? (details as QuotaDetails) : null;
}

/**
 * The sentence for a server code, with the quota's own counts when it gave
 * them, or `null` for a code {@link TURN_CODE_KEYS} does not word — the
 * caller then words the failure by its status (`NoProviderConnected` and
 * `AccountSuspended` are 403 refusals, not server faults).
 */
function describeCode(
  code: string | null | undefined,
  details: unknown,
  t: ApiErrorTranslate,
): string | null {
  if (!code) return null;
  if (code === 'QuotaExceeded') {
    const quota = asQuota(details);
    if (typeof quota?.limit_type === 'string') {
      return t(quotaKey({ limit_type: quota.limit_type, reason: quota.reason }), {
        current: typeof quota.current === 'number' ? quota.current : 0,
        limit: typeof quota.limit === 'number' ? quota.limit : 0,
      });
    }
    return t('chat.messageError');
  }
  const key = TURN_CODE_KEYS[code];
  return key === undefined ? null : t(key);
}

/**
 * What the athlete reads for a chat turn that did not finish, in their language.
 *
 * Every way a turn fails lands here: refused before it began (an HTTP error
 * body with a `code`), failed mid-stream (a `failed` frame with a `code`),
 * dropped by the idle stop, or lost in transit. Each is worded from the
 * shared catalogue. The server's `message` is never returned — it is English,
 * written for API callers, and a French thread once printed "The resource is
 * temporarily unavailable" verbatim (carnet#680).
 */
export function describeTurnFailure(
  err: unknown,
  opts: { online?: boolean; t: ApiErrorTranslate },
): string {
  const { t } = opts;
  if (err instanceof TurnIdleAbortedError) return t('chat.turnIdleAborted');
  if (err instanceof TurnFailedError) {
    return describeCode(err.code, err.details, t) ?? t(TURN_UNWORDED_KEY);
  }

  const refusal = (err as { response?: { data?: { code?: unknown; details?: unknown } } } | null)
    ?.response?.data;
  const coded =
    typeof refusal?.code === 'string' ? describeCode(refusal.code, refusal.details, t) : null;
  if (coded !== null) return coded;

  const { kind, status } = classifyApiError(err, { online: opts.online });
  const quotaSentence = describeQuotaRefusal(err, t);
  if (kind === 'quota' && quotaSentence !== undefined) return quotaSentence;
  if (kind === 'quota') return t('chat.messageError');
  // A 503 with no code is the same outage a coded one is.
  if (status === 503) return t('errors.generic');
  if (kind === 'unknown') return t(TURN_UNWORDED_KEY);
  return t(API_ERROR_KEYS[kind]);
}

/**
 * The server codes a turn fails with that a moment later may not: an upstream
 * outage, a shed request, a throttled provider. Every other code — a quota, a
 * refused permission, a lapsed session, a malformed question — answers the
 * same question the same way however often it is asked.
 */
const TRANSIENT_TURN_CODES: ReadonlySet<string> = new Set([
  'ResourceUnavailable',
  'ExternalServiceUnavailable',
  'ExternalServiceError',
  'ExternalRateLimited',
  'RateLimitExceeded',
]);

/**
 * Whether sending the same question again may succeed where this turn failed.
 *
 * True for a transient server code ({@link TRANSIENT_TURN_CODES}), an uncoded
 * 503, and a request that never reached the server or timed out on the way.
 * False for the idle stop — the server kept going, and its reply may already
 * be written — and for every refusal a retry would only repeat: a quota, a
 * permission, a session, a validation.
 */
export function isTurnFailureRetryable(err: unknown, opts: { online?: boolean } = {}): boolean {
  if (err instanceof TurnIdleAbortedError) return false;
  if (err instanceof TurnFailedError) {
    return err.code !== null && TRANSIENT_TURN_CODES.has(err.code);
  }
  const refusal = (err as { response?: { data?: { code?: unknown } } } | null)?.response?.data;
  if (typeof refusal?.code === 'string') return TRANSIENT_TURN_CODES.has(refusal.code);
  const { kind, status } = classifyApiError(err, { online: opts.online });
  return kind === 'network' || kind === 'offline' || kind === 'timeout' || status === 503;
}
