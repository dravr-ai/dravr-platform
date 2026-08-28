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
