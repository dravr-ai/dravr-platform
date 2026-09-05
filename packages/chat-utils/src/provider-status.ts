// ABOUTME: The chat header's provider line — which providers the coach can see, or that none are connected
// ABOUTME: Shared so the phone and the web say the same thing in the same state

/** The fields the line reads off a provider row, from either client's status call. */
export interface ProviderStatusRow {
  connected: boolean;
  display_name: string;
}

/**
 * The line a chat header shows when the thread has no group and no coach
 * handle to name: the providers the coach can actually read, or that there are
 * none.
 *
 * `loaded` gates it. Both clients start with an empty provider list, so a line
 * derived from that alone tells a connected athlete "no provider connected"
 * for as long as the status call is in flight.
 *
 * Shared because the phone said nothing at all in this state — an athlete whose
 * Strava session had died saw an ordinary header and a coach that had quietly
 * stopped citing their activities, with nothing anywhere saying why
 * (carnet#231).
 */
export function providerStatusLine(
  t: (key: string, vars?: Record<string, string>) => string,
  providers: readonly ProviderStatusRow[] | null | undefined,
  loaded: boolean,
): string | null {
  if (!loaded) return null;
  // Two rows can carry one name — the `sciotte` mirror and the `strava` OAuth
  // row are both "Strava" — and the line names the provider, not the backend.
  const names = [...new Set((providers ?? []).filter((p) => p.connected).map((p) => p.display_name))];
  if (names.length === 0) return t('chat.noProviderStatus');
  return t(names.length === 1 ? 'chat.providersConnectedOne' : 'chat.providersConnectedN', {
    providers: names.join(', '),
  });
}
