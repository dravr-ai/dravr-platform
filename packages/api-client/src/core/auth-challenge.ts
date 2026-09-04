// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Reads the RFC 6750 WWW-Authenticate challenge so a refusal says whether re-auth would help
// ABOUTME: The one parser — the interceptor and the turn transport both ask it, neither matches on prose

/**
 * What a refused request means for the session holding it.
 *
 * The distinction the status code alone cannot make. A 401 and a 403 carrying
 * `error="insufficient_scope"` are both "the credential you hold is not the
 * credential you need", and both recover by getting a new one. A bare 403 is
 * "you are who you say you are and the answer is still no" — re-authenticating
 * changes nothing, and logging the athlete out over it strands them in a login
 * loop, because the same refusal lands again on the other side.
 */
export type RefusalRecovery =
  /** Get a new credential: clear the session and drive re-authentication. */
  | 'reauthenticate'
  /** Nothing to retry. Tell the athlete what was refused and leave them signed in. */
  | 'none';

/**
 * The parsed `WWW-Authenticate` challenge, as far as a client cares.
 *
 * `scope` is the whole point of RFC 6750's `insufficient_scope`: it names the
 * grant to ask for, so a client that reads it can request exactly that instead
 * of guessing. Kept even though no first-party surface performs a step-up
 * today — it is what the server sent, and dropping it here would mean parsing
 * the header twice the day one does.
 */
export interface AuthChallenge {
  /** The RFC 6750 error code, e.g. `invalid_token`, `insufficient_scope`. */
  error?: string;
  /** Space-delimited scopes the resource requires, when the server named them. */
  scope?: string;
  /** RFC 9728 pointer at the protected-resource metadata document. */
  resourceMetadata?: string;
}

/** RFC 6750 §3.1's code for "your grant is too narrow", the one 403 re-auth fixes. */
const INSUFFICIENT_SCOPE = 'insufficient_scope';

/**
 * `auth-param` as RFC 7235 §2.1 spells it: `name=value` or `name="quoted"`,
 * comma-separated. Matched rather than split on commas because
 * `resource_metadata="https://…/a,b"` legally contains one.
 */
const AUTH_PARAM = /([A-Za-z_][A-Za-z0-9_-]*)\s*=\s*(?:"((?:[^"\\]|\\.)*)"|([^\s,]+))/g;

/**
 * Parse a `WWW-Authenticate` header value.
 *
 * Returns `null` for an absent, empty or non-Bearer challenge. Tolerant by
 * design: a header this client cannot read must degrade to "no challenge",
 * which is the conservative answer — it leaves the athlete signed in rather
 * than logging them out on a header it misunderstood.
 */
export function parseAuthChallenge(header: string | undefined | null): AuthChallenge | null {
  if (!header) return null;
  // Scheme is case-insensitive (RFC 7235 §2.1). Anything that is not Bearer
  // carries no RFC 6750 error code, so there is nothing here to read.
  if (!/^\s*Bearer\b/i.test(header)) return null;

  const challenge: AuthChallenge = {};
  // `lastIndex` is per-regex state and this literal is module-scoped, so a
  // stale index from a previous call would make the second read start mid-header.
  AUTH_PARAM.lastIndex = 0;
  for (let match = AUTH_PARAM.exec(header); match !== null; match = AUTH_PARAM.exec(header)) {
    const name = match[1].toLowerCase();
    // A quoted value keeps its escapes in group 2; a bare token lands in group 3.
    const value = match[2] !== undefined ? match[2].replace(/\\(.)/g, '$1') : (match[3] ?? '');
    if (name === 'error') challenge.error = value;
    else if (name === 'scope') challenge.scope = value;
    else if (name === 'resource_metadata') challenge.resourceMetadata = value;
  }

  return challenge.error === undefined &&
    challenge.scope === undefined &&
    challenge.resourceMetadata === undefined
    ? null
    : challenge;
}

/** A header bag that answers by method, as `Headers` and `AxiosHeaders` both do. */
interface HeaderGetter {
  get(name: string): unknown;
}

function hasGetter(bag: object): bag is HeaderGetter {
  return typeof (bag as HeaderGetter).get === 'function';
}

/**
 * Read one header from whichever bag this platform handed back.
 *
 * Three shapes reach this: `AxiosHeaders` and fetch's `Headers` both answer
 * `get()` case-insensitively, while axios also hands back a plain object when
 * a caller supplied its own headers — and mocked axios in tests always does.
 * Reading all three here keeps the callers from each guessing.
 *
 * `name` is given lowercase. A plain-object bag is still searched
 * case-insensitively, because header names are case-insensitive per RFC 7230
 * §3.2 — axios normalises them and so does React Native's XHR, but a bag this
 * client did not build has made no such promise.
 */
export function readHeader(bag: unknown, name: string): string | undefined {
  if (typeof bag !== 'object' || bag === null) return undefined;
  if (hasGetter(bag)) {
    const answered = bag.get(name);
    return typeof answered === 'string' ? answered : undefined;
  }
  const entry = Object.entries(bag as Record<string, unknown>).find(
    ([key]) => key.toLowerCase() === name
  );
  return typeof entry?.[1] === 'string' ? entry[1] : undefined;
}

/**
 * The mark the transport leaves on a refusal it has already acted on.
 *
 * `Symbol.for` rather than a private symbol or a plain field: the two apps
 * bundle this package by different means — Vite resolves it as ESM, Metro
 * through its own resolver — and a registry symbol is the same symbol across
 * however many module instances that produces. A plain field would collide with
 * a server-supplied body key; a private symbol would silently stop matching the
 * day one bundler duplicated the module.
 */
const DROVE_REAUTHENTICATION = Symbol.for('pierre.api-client.droveReauthentication');

/** Record that this refusal has already sent the athlete to sign in again. */
function markDroveReauthentication(error: unknown): void {
  if (typeof error !== 'object' || error === null) return;
  // Non-enumerable so the mark never reaches a log line, a JSON body or a
  // structural equality check — it is a note between the transport and the UI,
  // not part of the error.
  Object.defineProperty(error, DROVE_REAUTHENTICATION, {
    value: true,
    enumerable: false,
    configurable: true,
  });
}

/**
 * Has the transport already recovered this refusal by driving re-authentication?
 *
 * The question every app-wide error surface has to ask before it says anything.
 * A refusal that is signing the athlete out needs no permission toast — they
 * are on their way to the login form, and a message about a scope they no
 * longer hold is both wrong and unactionable.
 *
 * Answered from the transport's own verdict rather than re-derived. A consumer
 * that asked the question itself would need the status AND the challenge header
 * in hand and would have to agree with the interceptor about both; the two
 * clients each reached for a different proxy for this — one latched on the
 * sign-out event, the other could only detect the 401 half — and disagreed. One
 * decision, recorded where it was made.
 */
export function droveReauthentication(error: unknown): boolean {
  if (typeof error !== 'object' || error === null) return false;
  return (error as Record<symbol, unknown>)[DROVE_REAUTHENTICATION] === true;
}

/**
 * Act on a refusal: when re-authenticating is the recovery, mark the error,
 * clear the session and drive sign-in. Otherwise do nothing at all, which
 * leaves the error unmarked for an error surface to describe.
 *
 * Shared by the axios interceptor and the turn transport, which is the point —
 * `sendTurn` reads its body frame by frame and so cannot ride axios, and for as
 * long as it had its own idea of what to do about a refusal it did nothing at
 * all, on the endpoint an athlete spends the whole session in.
 */
export async function recoverFromRefusal(
  recovery: {
    authStorage: { clear(): Promise<void> };
    authFailure: { onAuthFailure(): void };
  },
  status: number,
  headerValue: string | undefined,
  error: unknown
): Promise<void> {
  if (refusalRecovery(status, headerValue) !== 'reauthenticate') return;
  markDroveReauthentication(error);
  await recovery.authStorage.clear();
  recovery.authFailure.onAuthFailure();
}

/**
 * Would re-authenticating fix this refusal?
 *
 * Reads the challenge header, never the body. The server puts the
 * machine-readable reason in `WWW-Authenticate` precisely so a client does not
 * have to match on a sentence — and this repo's 403 bodies are localised and
 * sanitised, so prose-matching would break on both axes at once.
 *
 * `headerValue` is whatever the platform handed back for `www-authenticate`;
 * axios lowercases every response header name, and React Native's XHR-backed
 * fetch does the same, so one lookup serves both surfaces.
 */
export function refusalRecovery(status: number, headerValue?: string | null): RefusalRecovery {
  // An unauthenticated request has always driven re-auth here, challenge or
  // not: 401 is the status the server uses when it could not identify the
  // caller at all, and RFC 6750 §3.1 explicitly permits omitting the error
  // code when a request "lacks any authentication information".
  if (status === 401) return 'reauthenticate';
  if (status !== 403) return 'none';
  return parseAuthChallenge(headerValue)?.error === INSUFFICIENT_SCOPE ? 'reauthenticate' : 'none';
}
