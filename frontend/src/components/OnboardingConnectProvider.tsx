// ABOUTME: First-run onboarding screen that forces a provider connection before the user reaches the dashboard
// ABOUTME: Sits in the shared OnboardingShell like every other step, the welcome as its heading

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useEffect, useRef, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { oauthApi, providersApi } from '../services/api';
import { QUERY_KEYS } from '../constants/queryKeys';
import { useAuth } from '../hooks/useAuth';
import { Button } from './ui';
import ProviderConnectionCards from './ProviderConnectionCards';
import ConnectPreview from './ConnectPreview';

// Dravr boreal-palette logo. Same dot-and-line motif as Login and
// PendingApproval; gradient IDs are suffixed with `-onboarding` so the
// embedded `<defs>` don't collide when multiple instances of the logo render
// in the same document tree.
import OnboardingShell from './OnboardingShell';
import { useTranslation } from '@pierre/i18n';
import { COACH_PLATFORM_PROVIDERS } from '../onboarding/steps';
import { PROVIDER_LINK_POLL_INTERVAL_MS } from '@pierre/shared-constants';

/** How long the awaiting-consent overlay waits for the OAuth popup to land. */
const AWAITING_OAUTH_TIMEOUT_MS = 90_000;

/**
 * Hard gate shown right after first login when the user has zero connected
 * providers. The same source of truth — `provider_connections` — drives the
 * backend's `NoProviderConnected` 403 on chat/coach/messaging, so this screen
 * cannot drift from server-side enforcement. Connecting a first provider
 * does not leave the screen: the list stays up with the new card marked
 * connected, so a second service is one tap away, and `onContinue` (offered
 * once a provider is connected) moves on to the next step.
 *
 * Skip is intentionally absent from the provider list — the agent has
 * nothing to reason about without provider data. Sign Out is offered as a
 * session escape (same convention as `PendingApproval`) so the user is never
 * trapped.
 *
 * A coach who does not train (`coachOnly`) is offered only the coaching
 * platforms, with no preview of athlete coaching: nothing reads their own
 * training, and skipping moves them on to their group.
 */
export default function OnboardingConnectProvider({
  userDisplayName,
  onContinueWithoutProvider,
  onContinue,
  coachOnly = false,
}: {
  userDisplayName?: string | null;
  onContinueWithoutProvider?: () => void;
  /** Leave the step; defined once at least one provider is connected. */
  onContinue?: () => void;
  coachOnly?: boolean;
}) {
  const { t } = useTranslation();
  const { logout } = useAuth();
  const [connectingProvider, setConnectingProvider] = useState<string | null>(null);
  const [connectError, setConnectError] = useState<string | null>(null);
  // Tracks an in-flight OAuth popup (Strava BYO path), by the name the
  // provider's card shows. We don't know whether the user has authorised yet
  // — the popup is in a separate window. This is displayed as an "awaiting
  // consent" overlay with a Cancel button and a 90s timeout so the user is
  // never stranded if they close the popup or walk away.
  const [awaitingOAuthFor, setAwaitingOAuthFor] = useState<string | null>(null);

  const launchOAuth = (provider: string, tosConsent: boolean) => {
    // Open the server's launch route directly. It is a same-origin page that
    // 302s to the provider, so the browser shows its normal loading state and
    // the window is never blank, and the open stays inside Safari's
    // user-gesture stack because nothing is awaited first.
    //
    // The previous shape — open `about:blank`, await the authorize URL, then
    // assign `location.href` — left the popup empty for the whole round trip.
    // On desktop that flashed past; on an iPhone it read as a broken connect,
    // and when the assignment did not take there was no signal at all.
    const popup = window.open(oauthApi.authorizeUrl(provider, { tosConsent }), '_blank');
    setConnectError(null);
    if (!popup) {
      // Popup blocked outright (strict mobile Safari). Same-tab navigation is
      // the documented fallback; OAuthCallback writes `pierre_oauth_result` to
      // localStorage regardless of which tab finishes the flow.
      window.location.href = oauthApi.authorizeUrl(provider, { tosConsent });
      return;
    }
    // No per-card spinner: the launch is complete the moment the window opens.
    // Success is observed through provider status (the card turns connected
    // and Continue appears), and holding a spinner here stranded the card
    // when the user closed the OAuth tab without finishing.
    setConnectingProvider(null);
  };

  // A provider that needs the athlete's own OAuth app (`own_app_required`)
  // has the cards open its setup in place before this runs, so first-run
  // users never need Settings, which this gate keeps them from.
  const handleConnectProvider = (provider: string, tosConsent: boolean) => {
    launchOAuth(provider, tosConsent);
  };

  // The same provider-status query the cards poll (shared cache). The cards
  // are unmounted under the awaiting-consent overlay, so this one asks while
  // the overlay is up. A new connection landing — a first or a second one —
  // closes it, putting the user back on the list with the card marked connected.
  const { data: providersData } = useQuery({
    queryKey: QUERY_KEYS.providers.status(),
    queryFn: () => providersApi.getProvidersStatus(),
    refetchInterval: awaitingOAuthFor ? PROVIDER_LINK_POLL_INTERVAL_MS : false,
  });
  const connectedCount = providersData?.providers?.filter((p) => p.connected).length ?? 0;
  const lastConnectedCount = useRef(connectedCount);
  useEffect(() => {
    if (connectedCount > lastConnectedCount.current) {
      setAwaitingOAuthFor(null);
      setConnectError(null);
    }
    lastConnectedCount.current = connectedCount;
  }, [connectedCount]);

  // Awaiting-OAuth timeout: 90s gives the user time to read, log in, MFA,
  // and approve in the popup. Beyond that we assume they bailed or got
  // blocked, surface a retry hint, and free the cards for another attempt.
  // Note: successful consent normally short-circuits this — the new
  // connection shows up in provider status and closes the overlay before the
  // timeout fires.
  useEffect(() => {
    if (!awaitingOAuthFor) return undefined;
    const provider = awaitingOAuthFor;
    const timer = window.setTimeout(() => {
      setAwaitingOAuthFor(null);
      setConnectError(
        t('onboarding.oauthNoReply', { provider, seconds: AWAITING_OAUTH_TIMEOUT_MS / 1000 }),
      );
    }, AWAITING_OAUTH_TIMEOUT_MS);
    return () => window.clearTimeout(timer);
  }, [awaitingOAuthFor, t]);

  if (awaitingOAuthFor) {
    return (
      <div className="min-h-dvh flex items-center justify-center py-12 px-4 sm:px-6 lg:px-8 bg-surface-container-low pad-safe-top">
        <div className="flex flex-col items-center gap-4 max-w-md text-center">
          <div className="pierre-spinner w-10 h-10 border-on-surface border-t-transparent" />
          <p className="text-sm text-on-surface">
            {t('onboarding.awaitingProviderConsent', { provider: awaitingOAuthFor })}
          </p>
          <p className="text-xs text-on-surface-variant">
            {t('onboarding.finishAuthInPopup', { provider: awaitingOAuthFor })}
          </p>
          <Button
            variant="secondary"
            onClick={() => setAwaitingOAuthFor(null)}
            className="mt-2"
          >
            {t('onboarding.cancelChooseOtherProvider')}
          </Button>
        </div>
      </div>
    );
  }

  // OnboardingShell's `pt-24` reserves room for the OnboardingProgress bar so
  // this taller provider-list card clears it; `my-auto` still centers it when
  // the viewport has room.
  return (
    <>
      <OnboardingShell
        heading={userDisplayName ? t('onboarding.welcomeNamed', { name: userDisplayName }) : t('onboarding.welcomeToDravr')}
      >
        <div className="flex flex-col">
          <p className="mt-3 text-sm text-on-surface-variant text-center mx-auto max-w-md">
            {coachOnly ? t('onboarding.connectCoachPlatformIntro') : t('onboarding.connectProviderIntro')}
          </p>

            <div className="mt-8">
              <ProviderConnectionCards
                onConnectProvider={handleConnectProvider}
                connectingProvider={connectingProvider}
                onOAuthLaunched={(provider) => setAwaitingOAuthFor(provider)}
                onlyProviders={coachOnly ? COACH_PLATFORM_PROVIDERS : undefined}
              />
            </div>

            {connectError && (
              <div
                role="alert"
                className="mt-4 rounded-lg border border-error/40 bg-error/10 px-4 py-3 text-sm text-error"
              >
                {connectError}
              </div>
            )}

            {onContinue && (
              <div className="mt-6 flex flex-col items-center gap-3" data-testid="onboarding-connect-continue">
                <p className="text-sm text-on-surface-variant text-center">
                  {t('onboarding.connectAnotherOrContinue')}
                </p>
                <Button variant="primary" onClick={onContinue}>
                  {t('onboarding.continueAfterConnect')}
                </Button>
              </div>
            )}

            {!coachOnly && <ConnectPreview />}

            <p className="mt-6 text-xs text-on-surface-variant text-center">
              {t('onboarding.credentialsEncryptedHint')}
            </p>

            {onContinueWithoutProvider && !onContinue && (
              <div className="mt-6 text-center">
                <button
                  type="button"
                  onClick={onContinueWithoutProvider}
                  className="text-sm font-medium text-on-surface-variant hover:text-on-surface underline-offset-2 hover:underline transition-colors"
                >
                  {t('onboarding.continueWithoutProvider')}
                </button>
                <p className="mt-1 text-xs text-on-surface-variant">
                  {coachOnly ? t('onboarding.connectCoachPlatformLaterHint') : t('onboarding.connectAnytimeHint')}
                </p>
              </div>
            )}

            <div className="mt-8 flex justify-center">
              <Button variant="tertiary" onClick={logout}>
                {t('common.logout')}
              </Button>
            </div>
        </div>
      </OnboardingShell>
    </>
  );
}
