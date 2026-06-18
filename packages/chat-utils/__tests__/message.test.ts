// ABOUTME: Tests for message content processing — context-prefix add/strip round-trip
// ABOUTME: Guards the slash-command exemption so chat commands reach the backend dispatcher

import { describe, it, expect } from 'vitest';
import { stripContextPrefix, buildOutgoingMessage } from '../src/message';

describe('stripContextPrefix', () => {
  it('removes a leading [Context:...] prefix', () => {
    expect(stripContextPrefix('[Context: I have connected Strava] Hello')).toBe('Hello');
  });

  it('leaves unprefixed text untouched', () => {
    expect(stripContextPrefix('Hello there')).toBe('Hello there');
  });
});

describe('buildOutgoingMessage', () => {
  it('returns slash commands verbatim even on the first turn with a connected provider', () => {
    // Regression: the provider-context prefix used to push the leading '/' off
    // position 0, so the backend dispatcher never recognized the command and it
    // fell through to the LLM. Commands must reach the API unprefixed.
    expect(
      buildOutgoingMessage('/context', { isFirstMessage: true, connectedProviders: 'Strava' }),
    ).toBe('/context');
    expect(
      buildOutgoingMessage('/coach list', {
        isFirstMessage: true,
        justConnectedProvider: 'Strava',
      }),
    ).toBe('/coach list');
  });

  it('prefixes a just-connected provider on a normal message', () => {
    expect(
      buildOutgoingMessage('how am I doing?', { isFirstMessage: true, justConnectedProvider: 'Garmin' }),
    ).toBe('[Context: I just connected my Garmin account successfully] how am I doing?');
  });

  it('prefixes connected providers on the first message only', () => {
    expect(
      buildOutgoingMessage('hi', { isFirstMessage: true, connectedProviders: 'Strava, Garmin' }),
    ).toBe('[Context: I have connected Strava, Garmin] hi');
    expect(
      buildOutgoingMessage('hi', { isFirstMessage: false, connectedProviders: 'Strava' }),
    ).toBe('hi');
  });

  it('returns content unchanged when there is no provider context', () => {
    expect(buildOutgoingMessage('hi', { isFirstMessage: true })).toBe('hi');
    expect(buildOutgoingMessage('hi', { isFirstMessage: true, connectedProviders: '' })).toBe('hi');
  });

  it('round-trips with stripContextPrefix for non-command messages', () => {
    const built = buildOutgoingMessage('hello', { isFirstMessage: true, connectedProviders: 'Strava' });
    expect(stripContextPrefix(built)).toBe('hello');
  });
});
