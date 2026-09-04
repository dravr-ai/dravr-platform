// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Proves a failed call is named by transport and status, never by the server's prose
// ABOUTME: The offline case is the one that used to read back to athletes as a wrong password

import { describe, it, expect } from 'vitest';
import { classifyApiError, describeApiError, API_ERROR_KEYS } from '../src/apiError';

/** An axios-shaped rejection that reached a server. */
const responded = (status: number, data?: Record<string, unknown>) => ({
  response: { status, data },
});

describe('classifyApiError', () => {
  it('names a dead radio "offline", not a rejected credential', () => {
    // The exact shape a fetch failure has: no response at all.
    const err = new Error('Network Error');
    expect(classifyApiError(err, { online: false })).toEqual({ kind: 'offline' });
  });

  it('separates a dead radio from a reachable-but-failing network', () => {
    const err = new Error('Network Error');
    expect(classifyApiError(err, { online: true }).kind).toBe('network');
    expect(classifyApiError(err, { online: false }).kind).toBe('offline');
  });

  it('reads an aborted connection as a timeout', () => {
    expect(classifyApiError({ code: 'ECONNABORTED' }, { online: true }).kind).toBe('timeout');
    expect(classifyApiError({ code: 'ETIMEDOUT' }, { online: true }).kind).toBe('timeout');
  });

  it('files OAuth invalid_grant as a credential problem despite its 400', () => {
    const c = classifyApiError(responded(400, { error: 'invalid_grant' }), { online: true });
    expect(c.kind).toBe('credentials');
    expect(c.status).toBe(400);
  });

  it('maps status codes to kinds', () => {
    const cases: Array<[number, string]> = [
      [401, 'unauthorized'],
      [403, 'forbidden'],
      [404, 'notFound'],
      [408, 'timeout'],
      [422, 'validation'],
      [400, 'validation'],
      [500, 'server'],
      [503, 'server'],
    ];
    for (const [status, kind] of cases) {
      expect(classifyApiError(responded(status), { online: true }).kind).toBe(kind);
    }
  });

  it('classifies a LOCALISED server error correctly', () => {
    // The regression this replaced: the login form matched
    // `errorMsg.includes('Invalid')`, so a French backend answering
    // "Identifiants invalides" contained no "Invalid" and a wrong password
    // fell through to the generic failure. Status is language-independent.
    const fr = classifyApiError(responded(401, { error: 'Identifiants invalides' }), {
      online: true,
    });
    expect(fr.kind).toBe('unauthorized');
    const de = classifyApiError(responded(401, { error: 'Ungültige Anmeldedaten' }), {
      online: true,
    });
    expect(de.kind).toBe('unauthorized');
  });

  it('surfaces the server detail it was given', () => {
    expect(
      classifyApiError(responded(422, { message: 'Password too short' }), { online: true }).detail,
    ).toBe('Password too short');
    // error_description is the OAuth spelling and must not be lost.
    expect(
      classifyApiError(responded(400, { error_description: 'code expired' }), { online: true })
        .detail,
    ).toBe('code expired');
  });

  it('reports no status when the request never landed', () => {
    expect(classifyApiError(new Error('boom'), { online: true }).status).toBeUndefined();
  });

  it('survives a non-object rejection', () => {
    expect(classifyApiError(undefined, { online: true }).kind).toBe('network');
    expect(classifyApiError(null, { online: false }).kind).toBe('offline');
  });

  it('reads a usage limit off the status or off the code', () => {
    // The conversation cap answers with the code on some paths, so a caller
    // that read only the status let a coach click fail silently.
    expect(classifyApiError(responded(429), { online: true }).kind).toBe('quota');
    expect(classifyApiError(responded(400, { code: 'QuotaExceeded' }), { online: true }).kind).toBe(
      'quota',
    );
  });

  it('carries the counted limit through, since which limit it was is the actionable part', () => {
    const c = classifyApiError(
      responded(429, {
        details: { limit_type: 'daily_messages', current: 50, limit: 50 },
      }),
      { online: true },
    );
    expect(c.quota).toEqual({ limit_type: 'daily_messages', current: 50, limit: 50 });
  });

  it('leaves quota unset on every other kind', () => {
    expect(classifyApiError(responded(403), { online: true }).quota).toBeUndefined();
    expect(classifyApiError(responded(500), { online: true }).quota).toBeUndefined();
  });

  it('classifies a refused chat turn identically to a refused axios call', () => {
    // A turn cannot ride axios — its body is read frame by frame — so
    // TurnRequestError wears the same `response` shape rather than making
    // every screen unwrap a second carrier. Same facts in, same kind out.
    class TurnShaped extends Error {
      constructor(
        private readonly status: number,
        private readonly body: unknown,
      ) {
        super('refused');
      }
      get response() {
        return { status: this.status, data: this.body };
      }
    }
    const turn = new TurnShaped(403, { code: 'PermissionDenied', message: 'Not the owner' });
    const axiosLike = responded(403, { code: 'PermissionDenied', message: 'Not the owner' });
    expect(classifyApiError(turn, { online: true })).toEqual(
      classifyApiError(axiosLike, { online: true }),
    );
    expect(classifyApiError(turn, { online: true }).kind).toBe('forbidden');
  });
});

describe('describeApiError', () => {
  // Echoes the key so the assertion names which sentence was chosen.
  const t = (key: string) => key;

  it('says offline even when the error carries a server detail', () => {
    const err = { response: { status: 500, data: { error: 'upstream exploded' } } };
    expect(describeApiError(err, { online: false, t, fallbackKey: 'x.y' })).toBe(
      API_ERROR_KEYS.server,
    );
    // With no response at all and no radio, the transport kind wins outright.
    expect(
      describeApiError(new Error('Network Error'), { online: false, t, fallbackKey: 'x.y' }),
    ).toBe(API_ERROR_KEYS.offline);
  });

  it('shows the server detail only for kinds where it describes the user', () => {
    expect(
      describeApiError(responded(422, { message: 'Key must be 32 chars' }), {
        online: true,
        t,
        fallbackKey: 'x.y',
      }),
    ).toBe('Key must be 32 chars');
    // A 500's internals are never shown.
    expect(
      describeApiError(responded(500, { message: 'NullPointerException at Foo.java:41' }), {
        online: true,
        t,
        fallbackKey: 'x.y',
      }),
    ).toBe(API_ERROR_KEYS.server);
  });

  it('falls back to the caller key when nothing else fits', () => {
    expect(
      describeApiError(responded(401), { online: true, t, fallbackKey: 'shell.intervalsLinkFailed' }),
    ).toBe('shell.intervalsLinkFailed');
  });

  it('names the specific limit that was hit, with its numbers', () => {
    // Interpolation is echoed so the assertion shows both the key chosen and
    // the counts handed to it.
    const withParams = (key: string, params?: Record<string, string | number>) =>
      params ? `${key}(${params.current}/${params.limit})` : key;
    const cases: Array<[string, string]> = [
      ['max_active_conversations', 'errors.conversationLimitReached'],
      ['daily_messages', 'errors.dailyMessageLimitReached'],
      ['daily_tokens', 'errors.dailyTokenLimitReached'],
      ['weekly_messages', 'errors.weeklyMessageLimitReached'],
    ];
    for (const [limitType, key] of cases) {
      expect(
        describeApiError(responded(429, { details: { limit_type: limitType, current: 3, limit: 5 } }), {
          online: true,
          t: withParams,
          fallbackKey: 'x.y',
        }),
      ).toBe(`${key}(3/5)`);
    }
  });

  it('falls back to the generic quota sentence for a limit it has no wording for', () => {
    const withParams = (key: string, params?: Record<string, string | number>) =>
      params ? `${key}(${params.current}/${params.limit})` : key;
    expect(
      describeApiError(responded(429, { details: { limit_type: 'monthly_widgets', limit: 9 } }), {
        online: true,
        t: withParams,
        fallbackKey: 'x.y',
      }),
    ).toBe(`${API_ERROR_KEYS.quota}(0/9)`);
  });

  it('gives a role refusal the sentence naming what was refused', () => {
    // The server stopped replacing this with one constant, so the athlete
    // reads the actual reason. A refusal that cannot say what it refused is
    // indistinguishable from a bug.
    expect(
      describeApiError(
        responded(403, {
          code: 'PermissionDenied',
          message: 'Group coaching requires a Professional or Enterprise plan',
        }),
        { online: true, t, fallbackKey: 'errors.forbidden' },
      ),
    ).toBe('Group coaching requires a Professional or Enterprise plan');
    // And when the server said nothing usable, the translated key — never the
    // axios sentence "Request failed with status code 403".
    expect(describeApiError(responded(403), { online: true, t, fallbackKey: 'errors.forbidden' })).toBe(
      'errors.forbidden',
    );
  });

  it('never returns a stringified Error', () => {
    // The two helpers this replaced ended in `${prefix}: ${err}`, which put
    // "AxiosError: Network Error" in front of athletes.
    const out = describeApiError(new Error('AxiosError: Network Error'), {
      online: true,
      t,
      fallbackKey: 'x.y',
    });
    expect(out).not.toContain('Error');
    expect(out).toBe(API_ERROR_KEYS.network);
  });
});
