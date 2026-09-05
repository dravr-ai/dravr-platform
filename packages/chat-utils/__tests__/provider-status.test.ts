// ABOUTME: The chat header's provider line — what the coach can see, or that nothing is connected
// ABOUTME: The phone rendered nothing in this state, so an athlete with a dead session was never told

import { describe, expect, it } from 'vitest';
import { providerStatusLine } from '../src/provider-status';

/** Renders the key and its interpolation, so a wrong key or a lost name shows. */
const t = (key: string, vars?: Record<string, string>) =>
  vars ? `${key}:${Object.values(vars).join('|')}` : key;

describe('providerStatusLine', () => {
  /**
   * The case carnet#231 is about. Web has said this for a while; the phone said
   * nothing at all, so a dead Strava session read as an ordinary quiet coach.
   */
  it('says no provider is connected when none is', () => {
    expect(providerStatusLine(t, [], true)).toBe('chat.noProviderStatus');
  });

  it('says no provider is connected when every row is disconnected', () => {
    expect(
      providerStatusLine(t, [{ connected: false, display_name: 'Strava' }], true),
    ).toBe('chat.noProviderStatus');
  });

  it('names the one connected provider in the singular', () => {
    expect(
      providerStatusLine(t, [{ connected: true, display_name: 'Strava' }], true),
    ).toBe('chat.providersConnectedOne:Strava');
  });

  it('names every connected provider in the plural, and drops the rest', () => {
    expect(
      providerStatusLine(
        t,
        [
          { connected: true, display_name: 'Strava' },
          { connected: false, display_name: 'Whoop' },
          { connected: true, display_name: 'Garmin' },
        ],
        true,
      ),
    ).toBe('chat.providersConnectedN:Strava, Garmin');
  });

  /**
   * The Strava data path has two backends — the `sciotte` mirror and the
   * `strava` OAuth row — and both display as "Strava". The header said
   * "Strava, Strava connected" to an athlete with one account.
   */
  it('names a provider once when two rows share its display name', () => {
    expect(
      providerStatusLine(
        t,
        [
          { connected: true, display_name: 'Strava' },
          { connected: true, display_name: 'Strava' },
        ],
        true,
      ),
    ).toBe('chat.providersConnectedOne:Strava');
  });

  /**
   * Both clients start with an empty list. Deriving the line from that alone
   * tells a connected athlete "no provider connected" for as long as the status
   * call is in flight, which is a wrong sentence, not a slow one.
   */
  it('says nothing until the status call has answered', () => {
    expect(providerStatusLine(t, [], false)).toBeNull();
    expect(
      providerStatusLine(t, [{ connected: true, display_name: 'Strava' }], false),
    ).toBeNull();
  });

  it('treats a missing list as an answered empty one', () => {
    expect(providerStatusLine(t, null, true)).toBe('chat.noProviderStatus');
    expect(providerStatusLine(t, undefined, true)).toBe('chat.noProviderStatus');
  });
});
