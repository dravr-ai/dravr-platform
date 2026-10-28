// ABOUTME: The chat header's provider line — what the coach can see, or that nothing is connected
// ABOUTME: The phone rendered nothing in this state, so an athlete with a dead session was never told

import { describe, expect, it } from 'vitest';
import fr from '../../i18n/src/locales/fr/translation.json';
import { PROVIDERS_CONNECTED_KEYS } from '../src/count-keys';
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
    expect(providerStatusLine(t, [], true, 'en')).toBe('chat.noProviderStatus');
  });

  it('says no provider is connected when every row is disconnected', () => {
    expect(
      providerStatusLine(t, [{ connected: false, display_name: 'Strava' }], true, 'en'),
    ).toBe('chat.noProviderStatus');
  });

  it('names the one connected provider in the singular', () => {
    expect(
      providerStatusLine(t, [{ connected: true, display_name: 'Strava' }], true, 'en'),
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
        'en',
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
        'en',
      ),
    ).toBe('chat.providersConnectedOne:Strava');
  });

  /**
   * Both clients start with an empty list. Deriving the line from that alone
   * tells a connected athlete "no provider connected" for as long as the status
   * call is in flight, which is a wrong sentence, not a slow one.
   */
  it('says nothing until the status call has answered', () => {
    expect(providerStatusLine(t, [], false, 'en')).toBeNull();
    expect(
      providerStatusLine(t, [{ connected: true, display_name: 'Strava' }], false, 'en'),
    ).toBeNull();
  });

  it('treats a missing list as an answered empty one', () => {
    expect(providerStatusLine(t, null, true, 'en')).toBe('chat.noProviderStatus');
    expect(providerStatusLine(t, undefined, true, 'en')).toBe('chat.noProviderStatus');
  });

  /**
   * A flagged connection is still `connected` on the wire, but nothing new
   * syncs through it: the header said "Strava connected" above the banner
   * saying Strava had to be reconnected (carnet#647).
   */
  it('never names a connection flagged needs_reauth as connected', () => {
    expect(
      providerStatusLine(t, [{ connected: true, display_name: 'Strava', needs_reauth: true }], true, 'en'),
    ).toBe('providers.reconnectNeeded');
    expect(
      providerStatusLine(
        t,
        [
          { connected: true, display_name: 'Strava', needs_reauth: true },
          { connected: true, display_name: 'WHOOP', needs_reauth: false },
        ],
        true,
        'en',
      ),
    ).toBe('chat.providersConnectedOne:WHOOP');
    expect(
      providerStatusLine(t, [{ connected: false, display_name: 'COROS', needs_reauth: true }], true, 'en'),
    ).toBe('chat.noProviderStatus');
  });

  /**
   * The participle agrees with how many providers are named, and the choice
   * goes through countKey, so French reads « Strava connecté » for one and
   * « Strava, Garmin connectés » for two (carnet#1212).
   */
  it('picks the participle through countKey in French, from the pair count-keys holds', () => {
    const one = [{ connected: true, display_name: 'Strava' }];
    const two = [...one, { connected: true, display_name: 'Garmin' }];
    expect(providerStatusLine(t, one, true, 'fr')).toBe(`${PROVIDERS_CONNECTED_KEYS.one}:Strava`);
    expect(providerStatusLine(t, two, true, 'fr-CA')).toBe(`${PROVIDERS_CONNECTED_KEYS.many}:Strava, Garmin`);
    // Rendered against the shipped French corpus, the participle agrees.
    const frT = (key: string, vars?: Record<string, string>) => {
      const [group, leaf] = key.split('.');
      const template = (fr as unknown as Record<string, Record<string, string>>)[group]?.[leaf] ?? key;
      return template.replace(/\{\{(\w+)\}\}/g, (_, name: string) => vars?.[name] ?? '');
    };
    expect(providerStatusLine(frT, one, true, 'fr')).toBe('Strava connecté');
    expect(providerStatusLine(frT, two, true, 'fr')).toBe('Strava, Garmin connectés');
  });
});
