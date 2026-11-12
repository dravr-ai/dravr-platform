// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins how a login takeover's messages are read — its events, its last answer, the phone bridge, text chunks, the token's provider
// ABOUTME: Red if a refusal stops reading as the HTTP client's error, an ended takeover reads as a failure, or a chunk splits a character

import { describe, expect, it } from 'vitest';
import {
  parseTakeoverBridgeMessage,
  parseTakeoverEvent,
  takeoverResult,
  takeoverTarget,
  takeoverTextChunks,
  takeoverTokenProvider,
} from '../src/sciotteTakeover';
import { isSignInPhaseLive } from '../src/useSignInAttempt';
import { refusalReason } from '../src/apiError';

const decode = (b64: string) => Buffer.from(b64, 'base64').toString('utf8');
const encode = (json: string) => Buffer.from(json, 'utf8').toString('base64url');

describe('takeoverResult', () => {
  it('reads a connected answer as the login answer it is', () => {
    expect(
      takeoverResult({ type: 'outcome', status_code: 200, body: { status: 'connected', provider: 'sciotte_garmin' } }),
    ).toEqual({ kind: 'answer', data: { status: 'connected', provider: 'sciotte_garmin' } });
  });

  it('reads a credential rejection as an answer, not a refusal', () => {
    expect(takeoverResult({ type: 'outcome', status_code: 200, body: { status: 'failed', error: 'bad' } })).toEqual({
      kind: 'answer',
      data: { status: 'failed', error: 'bad' },
    });
  });

  it("shapes a refusal as the HTTP client's error, which refusalReason reads", () => {
    const result = takeoverResult({
      type: 'outcome',
      status_code: 409,
      body: { code: 'Conflict', message: 'x', details: { reason: 'login_cancelled' } },
    });
    expect(result.kind).toBe('refusal');
    if (result.kind !== 'refusal') return;
    expect(result.error.response.status).toBe(409);
    expect(refusalReason(result.error)).toBe('login_cancelled');
  });

  it('reads takeover_ended, an empty body and a body with no status as ended', () => {
    expect(
      takeoverResult({ type: 'outcome', status_code: 400, body: { details: { reason: 'takeover_ended' } } }).kind,
    ).toBe('ended');
    expect(takeoverResult({ type: 'outcome', status_code: 500, body: null }).kind).toBe('ended');
    expect(takeoverResult({ type: 'outcome', status_code: 200, body: {} }).kind).toBe('ended');
  });
});

describe('parseTakeoverEvent', () => {
  it('reads each event the server sends', () => {
    expect(parseTakeoverEvent('{"type":"status","state":"ready","width":390,"height":844}')).toEqual({
      type: 'status',
      state: 'ready',
      width: 390,
      height: 844,
    });
    expect(parseTakeoverEvent('{"type":"popup_refused"}')).toEqual({ type: 'popup_refused' });
    expect(parseTakeoverEvent('{"type":"navigation_reset"}')).toEqual({ type: 'navigation_reset' });
    expect(parseTakeoverEvent('{"type":"outcome","status_code":200,"body":{"status":"connected"}}')).toEqual({
      type: 'outcome',
      status_code: 200,
      body: { status: 'connected' },
    });
  });

  it('drops anything else', () => {
    expect(parseTakeoverEvent('not json')).toBeNull();
    expect(parseTakeoverEvent('{"type":"status","state":"loading","width":1,"height":1}')).toBeNull();
    expect(parseTakeoverEvent('{"type":"something_new"}')).toBeNull();
    expect(parseTakeoverEvent('[1]')).toBeNull();
  });
});

describe('parseTakeoverBridgeMessage', () => {
  it('reads connected, and closed with or without its answer', () => {
    expect(parseTakeoverBridgeMessage('{"type":"connected"}')).toEqual({ type: 'connected' });
    expect(parseTakeoverBridgeMessage('{"type":"connected","follow_on":"garmin"}')).toEqual({
      type: 'connected',
      follow_on: 'garmin',
    });
    expect(parseTakeoverBridgeMessage('{"type":"closed"}')).toEqual({ type: 'closed' });
    expect(
      parseTakeoverBridgeMessage('{"type":"closed","outcome":{"type":"outcome","status_code":200,"body":{"status":"failed","error":"x"}}}'),
    ).toEqual({ type: 'closed', outcome: { type: 'outcome', status_code: 200, body: { status: 'failed', error: 'x' } } });
  });

  it('drops anything else', () => {
    expect(parseTakeoverBridgeMessage('{"type":"token","token":"x"}')).toBeNull();
    expect(parseTakeoverBridgeMessage('{')).toBeNull();
  });
});

describe('takeoverTextChunks', () => {
  it('keeps short text whole, repeated letters included', () => {
    expect(takeoverTextChunks('aaabbb')).toEqual(['aaabbb']);
  });

  it('never splits a character across chunks', () => {
    const chunks = takeoverTextChunks('éé😀a', 4);
    expect(chunks).toEqual(['éé', '😀', 'a']);
    expect(chunks.join('')).toBe('éé😀a');
  });

  it('cuts a long paste into pieces the server takes', () => {
    const text = 'x'.repeat(2500);
    const chunks = takeoverTextChunks(text);
    expect(chunks.map((c) => c.length)).toEqual([1024, 1024, 452]);
    expect(chunks.join('')).toBe(text);
  });

  it('sends nothing for empty text', () => {
    expect(takeoverTextChunks('')).toEqual([]);
  });
});

describe('takeoverTokenProvider and takeoverTarget', () => {
  it("reads the token's provider for display", () => {
    const token = `${encode('{"alg":"HS256"}')}.${encode('{"tgt":"sciotte_garmin","sub":"u"}')}.sig`;
    expect(takeoverTokenProvider(token, decode)).toBe('sciotte_garmin');
  });

  it('names no provider for a token that is not a JWT carrying one', () => {
    expect(takeoverTokenProvider('opaque', decode)).toBeNull();
    expect(takeoverTokenProvider(`a.${encode('{"sub":"u"}')}.c`, decode)).toBeNull();
    expect(takeoverTokenProvider('a.%%%.c', decode)).toBeNull();
  });

  it('maps a backend name or a target name to its target', () => {
    expect(takeoverTarget('sciotte_garmin')).toBe('garmin');
    expect(takeoverTarget('sciotte')).toBe('strava');
    expect(takeoverTarget('coros')).toBe('coros');
    expect(takeoverTarget('toString')).toBeNull();
    expect(takeoverTarget('whoop')).toBeNull();
  });
});

describe('the takeover phase', () => {
  it('is live, so closing the window cancels the attempt it holds', () => {
    expect(isSignInPhaseLive('takeover')).toBe(true);
  });
});
