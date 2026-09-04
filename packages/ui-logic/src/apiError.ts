// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Classifies a failed API call into a kind the UI can translate, from status not prose
// ABOUTME: One classifier, so an offline device never reads back as a rejected password

/**
 * What went wrong, in terms a screen can turn into a sentence.
 *
 * Deliberately a closed set of kinds rather than a message: the caller owns
 * the wording, because the same failure reads differently on a login form
 * ("wrong password") and on a settings page ("your session expired"). The
 * classifier only says which of those happened.
 */
export type ApiErrorKind =
  | 'offline'
  | 'network'
  | 'timeout'
  | 'credentials'
  | 'unauthorized'
  /**
   * A 403. Always a refusal re-authenticating cannot fix: the transport
   * intercepts the one 403 that re-auth *does* fix — RFC 6750's
   * `insufficient_scope` — and drives sign-in before the rejection ever
   * reaches a screen. So a `forbidden` here means the athlete is signed in
   * correctly and the answer is still no, and the wording must never suggest
   * signing in again.
   */
  | 'forbidden'
  | 'notFound'
  | 'validation'
  /** Over a usage limit (429). Recovers by waiting or by freeing something up. */
  | 'quota'
  | 'server'
  | 'unknown';

export interface ClassifiedApiError {
  kind: ApiErrorKind;
  /**
   * Detail the SERVER supplied, when it actually answered. Never a stringified
   * Error — the two hand-rolled extractors this replaces both ended with
   * `${prefix}: ${err}`, which put "AxiosError: Network Error" in front of
   * athletes and leaked internals into the UI.
   */
  detail?: string;
  /** Present only when a response came back. Absent means the request never landed. */
  status?: number;
  /**
   * The limit the server counted, when it named one. Only ever set on a
   * `quota` kind, and the reason that kind exists: which limit was hit is the
   * whole actionable part, since "delete a conversation" and "resets tomorrow"
   * are different instructions.
   */
  quota?: QuotaDetails;
}

/**
 * A translator, as either client's `t` is.
 *
 * Takes the interpolation bag because a quota sentence has to name the numbers
 * it hit. A caller holding a bare `(key) => string` still satisfies this — TS
 * accepts a function that ignores an argument — so the narrower callers that
 * predate the quota path need no change.
 */
export type ApiErrorTranslate = (key: string, params?: Record<string, string | number>) => string;

/** What the server says when a caller is over one of its usage limits. */
interface QuotaDetails {
  limit_type?: string;
  current?: number;
  limit?: number;
}

/**
 * The error shape this module reads, duck-typed rather than imported.
 *
 * Deliberately not `AxiosError`: this package must stay usable from React
 * Native, from a plain `fetch` caller, and from a test that hands over a bare
 * object literal. `TurnRequestError` — the refusal a chat turn produces, which
 * cannot ride axios because the body is read frame by frame — presents this
 * same `response` shape for exactly this reason, so one classifier reads both
 * carriers and no screen unwraps a transport.
 */
interface AxiosShape {
  code?: string;
  message?: string;
  response?: {
    status?: number;
    data?: {
      message?: string;
      error?: string;
      error_description?: string;
      code?: string;
      details?: QuotaDetails;
    };
  };
}

/**
 * OAuth's name for a rejected credential. The server returns it with a 400,
 * not a 401, so status alone would file it as a validation error and the login
 * form would tell the athlete to check their formatting.
 */
const INVALID_GRANT = 'invalid_grant';

/**
 * Classify a thrown API error.
 *
 * `online` is threaded in rather than read from `navigator` here so the
 * function stays pure and testable, and so React Native — which has no
 * `navigator.onLine` — can pass NetInfo's answer instead.
 *
 * Classification is by status code and transport state. The previous login
 * path matched on server prose (`errorMsg.includes('Invalid')`), which broke
 * the moment the server localised its own errors: a French backend saying
 * "Identifiants invalides" contains no "Invalid", so a wrong password fell
 * through to the generic failure.
 */
export function classifyApiError(
  err: unknown,
  opts: { online?: boolean } = {},
): ClassifiedApiError {
  const e = (err ?? {}) as AxiosShape;
  const data = e.response?.data;
  const detail = data?.message || data?.error_description || data?.error || undefined;
  const status = e.response?.status;

  // No response at all: the request never reached a server. Keyed off the
  // presence of `response`, not of `status` — a rejection that carries a body
  // but no status code did reach a server, and reading it as a dead network
  // would tell an athlete with a rejected password to check their signal.
  if (e.response === undefined) {
    if (opts.online === false) {
      return { kind: 'offline' };
    }
    if (e.code === 'ECONNABORTED' || e.code === 'ETIMEDOUT') {
      return { kind: 'timeout' };
    }
    return { kind: 'network' };
  }

  if (status === undefined) {
    return { kind: 'unknown', detail };
  }

  if (data?.error === INVALID_GRANT) {
    return { kind: 'credentials', detail, status };
  }
  if (status === 401) {
    return { kind: 'unauthorized', detail, status };
  }
  if (status === 403) {
    return { kind: 'forbidden', detail, status };
  }
  if (status === 404) {
    return { kind: 'notFound', detail, status };
  }
  if (status === 408) {
    return { kind: 'timeout', detail, status };
  }
  // Before the status ladder, because the server's own name for the condition
  // is more specific than a status inferred from it. `QuotaExceeded` maps to
  // 429 today, so this reads as belt and braces — but tested below the 400 arm
  // it would be unreachable the moment that mapping moved, which is exactly
  // when a caller would need it.
  if (status === 429 || data?.code === 'QuotaExceeded') {
    return { kind: 'quota', detail, status, quota: data?.details };
  }
  if (status === 422 || status === 400) {
    return { kind: 'validation', detail, status };
  }
  if (status >= 500) {
    return { kind: 'server', detail, status };
  }
  return { kind: 'unknown', detail, status };
}

/**
 * The i18n key each kind reads as by default.
 *
 * A screen that needs different wording maps the kind itself — the login form
 * reads `unauthorized` as a bad password rather than an expired session — but
 * everything else gets a translated sentence without restating the table.
 */
export const API_ERROR_KEYS: Record<ApiErrorKind, string> = {
  offline: 'errors.offline',
  network: 'errors.network',
  timeout: 'errors.timeout',
  credentials: 'auth.invalidCredentials',
  unauthorized: 'errors.unauthorized',
  forbidden: 'errors.forbidden',
  notFound: 'errors.notFound',
  validation: 'errors.validation',
  quota: 'errors.usageQuotaReached',
  server: 'errors.serverError',
  unknown: 'errors.unknown',
};

/**
 * The sentence for the specific limit the caller hit.
 *
 * Which limit it was is the only actionable part — "delete a conversation" and
 * "resets tomorrow" are different instructions — so the generic quota key is
 * the fallback, not the answer.
 */
function quotaKey(limitType: string | undefined): string {
  switch (limitType) {
    case 'max_active_conversations':
      return 'errors.conversationLimitReached';
    case 'daily_messages':
      return 'errors.dailyMessageLimitReached';
    case 'daily_tokens':
      return 'errors.dailyTokenLimitReached';
    case 'weekly_messages':
      return 'errors.weeklyMessageLimitReached';
    default:
      return API_ERROR_KEYS.quota;
  }
}

/**
 * Whether the server's own message is safe to show instead of the generic one.
 *
 * Only for the kinds where the server is describing something the athlete did
 * — a validation complaint is worth reading, a 500's internals are not.
 */
export function prefersServerDetail(kind: ApiErrorKind): boolean {
  return (
    kind === 'validation' ||
    kind === 'forbidden' ||
    kind === 'notFound' ||
    // An uncategorised status that still carried a sentence: the server is
    // explaining something we have no better words for. A 5xx is deliberately
    // NOT here — that is where stack traces and internals leak from.
    kind === 'unknown'
  );
}

/**
 * A translated sentence for a failed call, in one line.
 *
 * `fallbackKey` is what the screen wants said when the failure is specific to
 * it and none of the transport kinds apply — "could not link Intervals.icu"
 * rather than a bare "something went wrong".
 *
 * Transport kinds always win over the server's detail: a device with no radio
 * has to read as offline no matter what the last cached response said. This
 * replaces two byte-identical `extractErrorMessage` helpers that both ended in
 * `` `${prefix}: ${err}` ``, printing "AxiosError: Network Error" to athletes.
 */
export function describeApiError(
  err: unknown,
  opts: { online?: boolean; t: ApiErrorTranslate; fallbackKey: string },
): string {
  const { kind, detail, quota } = classifyApiError(err, { online: opts.online });
  if (kind === 'offline' || kind === 'network' || kind === 'timeout' || kind === 'server') {
    return opts.t(API_ERROR_KEYS[kind]);
  }
  // A limit the server counted is described from its own numbers, translated —
  // never from its prose, which is the one place the counts are not localised.
  if (quota?.limit_type !== undefined) {
    return opts.t(quotaKey(quota.limit_type), {
      current: quota.current ?? 0,
      limit: quota.limit ?? 0,
    });
  }
  if (detail && prefersServerDetail(kind)) {
    return detail;
  }
  return opts.t(opts.fallbackKey);
}
