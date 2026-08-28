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
  | 'forbidden'
  | 'notFound'
  | 'validation'
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
}

interface AxiosShape {
  code?: string;
  message?: string;
  response?: {
    status?: number;
    data?: { message?: string; error?: string; error_description?: string };
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
  server: 'errors.serverError',
  unknown: 'errors.unknown',
};

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
  opts: { online?: boolean; t: (key: string) => string; fallbackKey: string },
): string {
  const { kind, detail } = classifyApiError(err, { online: opts.online });
  if (kind === 'offline' || kind === 'network' || kind === 'timeout' || kind === 'server') {
    return opts.t(API_ERROR_KEYS[kind]);
  }
  if (detail && prefersServerDetail(kind)) {
    return detail;
  }
  return opts.t(opts.fallbackKey);
}
