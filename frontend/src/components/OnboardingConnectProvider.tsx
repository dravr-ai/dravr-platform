// ABOUTME: First-run onboarding screen that forces a provider connection before the user reaches the dashboard
// ABOUTME: Mirrors the PendingApproval system-page pattern (boreal-hero-gradient accent + DravrLogo + Card chrome)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useEffect, useState } from 'react';
import { oauthApi } from '../services/api';
import { useAuth } from '../hooks/useAuth';
import { Button, Card } from './ui';
import ProviderConnectionCards from './ProviderConnectionCards';
import OAuthAppSetupModal from './OAuthAppSetupModal';

// Dravr boreal-palette logo. Same dot-and-line motif as Login and
// PendingApproval; gradient IDs are suffixed with `-onboarding` so the
// embedded `<defs>` don't collide when multiple instances of the logo render
// in the same document tree.
import { DravrLogo } from './DravrLogo';

/**
 * Hard gate shown right after first login when the user has zero connected
 * providers. The same source of truth — `provider_connections` — drives the
 * backend's `NoProviderConnected` 403 on chat/coach/messaging, so this screen
 * cannot drift from server-side enforcement. Once the user connects any
 * provider, the OAuth callback in `App.tsx` invalidates the onboarding-status
 * query and the redirect flips to the dashboard.
 *
 * Skip is intentionally absent from the provider list — the LLM coach has
 * nothing to reason about without provider data. Sign Out is offered as a
 * session escape (same convention as `PendingApproval`) so the user is never
 * trapped.
 */
export default function OnboardingConnectProvider({
  userDisplayName,
  onContinueWithoutProvider,
}: {
  userDisplayName?: string | null;
  onContinueWithoutProvider?: () => void;
}) {
  const { logout } = useAuth();
  const [connectingProvider, setConnectingProvider] = useState<string | null>(null);
  // Bridges the gap between Sciotte success and the App-level route flip:
  // ProviderConnectionCards fires `onProviderConnected`, this flag flips on,
  // and the page renders a fullscreen spinner instead of the static
  // "Connected" badge while the onboarding-status query refetches.
  const [justConnected, setJustConnected] = useState(false);
  const [connectError, setConnectError] = useState<string | null>(null);
  // Tracks an in-flight OAuth popup (Strava BYO path). We don't know whether
  // the user has authorised yet — the popup is in a separate window. This is
  // displayed as an "awaiting consent" overlay with a Cancel button and a 90s
  // timeout so the user is never stranded if they close the popup or walk
  // away.
  const [awaitingOAuthFor, setAwaitingOAuthFor] = useState<string | null>(null);
  // WHOOP is BYO-OAuth-app: the user must register a Whoop developer app and
  // store its client_id/secret before the OAuth dance can run. Rather than
  // bouncing first-run users to Settings (which they can't reach behind this
  // gate), we open the setup modal in-place and continue with OAuth as soon as
  // they save.
  const [showWhoopSetup, setShowWhoopSetup] = useState(false);

  const launchOAuth = async (provider: string) => {
    // Mobile Safari requires window.open to fire inside the synchronous
    // user-gesture call stack. Pre-open a blank window so the popup permission
    // is captured before the async authorize-URL fetch. (For Whoop's
    // post-save path we accept the same-tab fallback — the modal close already
    // ate the user-gesture.)
    const popup = window.open('about:blank', '_blank');
    setConnectingProvider(provider);
    setConnectError(null);
    try {
      const authUrl = await oauthApi.getAuthorizeUrlForProvider(provider);
      if (popup && !popup.closed) {
        popup.location.href = authUrl;
      } else {
        window.location.href = authUrl;
        return;
      }
      // Clear the per-card spinner now that the OAuth tab is loading. Success
      // is observed at the App level (onboarding-status invalidation flips to
      // the dashboard); keeping the spinner held here strands the card if the
      // user closes the OAuth tab without finishing.
      setConnectingProvider(null);
    } catch (error) {
      if (popup && !popup.closed) {
        popup.close();
      }
      console.error(`Failed to get OAuth URL for ${provider}:`, error);
      setConnectingProvider(null);
      if (provider === 'whoop') {
        // No BYO app registered yet — open the in-place setup modal so the
        // first-run user never needs to navigate to Settings.
        setShowWhoopSetup(true);
        return;
      }
      setConnectError(`Couldn’t start the ${provider} connect flow. Please try again.`);
    }
  };

  const handleConnectProvider = (provider: string) => {
    if (provider === 'whoop') {
      // Skip the speculative OAuth init for Whoop — open the setup modal
      // directly. The modal pre-populates from any existing app if present, so
      // returning users see their saved client_id and only need to re-enter
      // the secret (which we don't keep around for security).
      setShowWhoopSetup(true);
      return;
    }
    void launchOAuth(provider);
  };

  // Safety net: if the App-level route flip never happens (provider-status
  // refetch races, OAuth callback never lands, etc.) auto-revert to the cards
  // so the user isn't pinned on the spinner forever. 30s is plenty for a
  // successful refetch; anything longer is a hung state we should escape.
  useEffect(() => {
    if (!justConnected) return undefined;
    const timer = window.setTimeout(() => {
      setJustConnected(false);
      setConnectError(
        'Couldn’t confirm the connection. If you completed the connect flow, refresh the page; otherwise try again.',
      );
    }, 30_000);
    return () => window.clearTimeout(timer);
  }, [justConnected]);

  // Awaiting-OAuth timeout: 90s gives the user time to read, log in, MFA,
  // and approve in the popup. Beyond that we assume they bailed or got
  // blocked, surface a retry hint, and free the cards for another attempt.
  // Note: successful consent normally short-circuits this — the OAuth
  // callback lands in `App.tsx` and the route flips before the timeout
  // fires.
  useEffect(() => {
    if (!awaitingOAuthFor) return undefined;
    const provider = awaitingOAuthFor;
    const timer = window.setTimeout(() => {
      setAwaitingOAuthFor(null);
      setConnectError(
        `Didn’t hear back from ${provider.charAt(0).toUpperCase() + provider.slice(1)} within 90 seconds. If the popup is still open, finish authorising there; otherwise try again.`,
      );
    }, 90_000);
    return () => window.clearTimeout(timer);
  }, [awaitingOAuthFor]);

  if (justConnected) {
    return (
      <div className="min-h-dvh flex items-center justify-center py-12 px-4 sm:px-6 lg:px-8 bg-surface-container-low">
        <div className="flex flex-col items-center gap-4">
          <div className="pierre-spinner w-10 h-10 border-on-surface border-t-transparent" />
          <p className="text-sm text-on-surface-variant font-label">
            Provider connected — preparing your dashboard…
          </p>
        </div>
      </div>
    );
  }

  if (awaitingOAuthFor) {
    const friendlyName = awaitingOAuthFor.charAt(0).toUpperCase() + awaitingOAuthFor.slice(1);
    return (
      <div className="min-h-dvh flex items-center justify-center py-12 px-4 sm:px-6 lg:px-8 bg-surface-container-low">
        <div className="flex flex-col items-center gap-4 max-w-md text-center">
          <div className="pierre-spinner w-10 h-10 border-on-surface border-t-transparent" />
          <p className="text-sm text-on-surface font-label">
            Awaiting {friendlyName} consent…
          </p>
          <p className="text-xs text-on-surface-variant">
            Finish the authorisation in the popup window. We&apos;ll route you to the
            dashboard automatically once {friendlyName} confirms.
          </p>
          <Button
            variant="secondary"
            onClick={() => setAwaitingOAuthFor(null)}
            className="mt-2"
          >
            Cancel and try a different provider
          </Button>
        </div>
      </div>
    );
  }

  // `pt-20` reserves a top safe-area for the fixed OnboardingProgress bar so
  // this taller provider-list card clears it; `my-auto` still centers it when
  // the viewport has room.
  return (
    <div className="min-h-dvh flex flex-col items-center px-4 sm:px-6 lg:px-8 py-12 pt-20 bg-surface-container-low">
      <div className="max-w-2xl w-full my-auto">
        <Card className="overflow-hidden">
          {/* Gradient accent bar — matches the PendingApproval brand moment. */}
          <div className="h-1 w-full boreal-hero-gradient" />

          <div className="px-8 py-10">
            <div className="flex flex-col items-center text-center">
              <DravrLogo size={64} />

              <h1 className="mt-6 font-display font-semibold text-3xl text-on-surface">
                {userDisplayName ? `Welcome, ${userDisplayName}` : 'Welcome to Dravr'}
              </h1>

              <p className="mt-3 text-sm text-on-surface-variant max-w-md font-label">
                Connect a fitness service to get started. Dravr coaches you on
                the activities your provider already tracks — without one,
                there&apos;s nothing for the model to read.
              </p>
            </div>

            <div className="mt-8">
              <ProviderConnectionCards
                onConnectProvider={handleConnectProvider}
                connectingProvider={connectingProvider}
                onProviderConnected={() => setJustConnected(true)}
                onOAuthLaunched={(provider) => setAwaitingOAuthFor(provider)}
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

            <p className="mt-6 text-xs text-on-surface-variant text-center">
              Your credentials are encrypted at rest and used only to fetch your activity data.
            </p>

            {onContinueWithoutProvider && (
              <div className="mt-6 text-center">
                <button
                  type="button"
                  onClick={onContinueWithoutProvider}
                  className="text-sm font-medium text-on-surface-variant hover:text-on-surface underline-offset-2 hover:underline transition-colors"
                >
                  Continue without connecting &rarr;
                </button>
                <p className="mt-1 text-xs text-on-surface-variant">
                  You can connect anytime &mdash; your coach needs a provider to read your activity.
                </p>
              </div>
            )}

            <div className="mt-8">
              <Button
                variant="secondary"
                onClick={logout}
                className="w-full"
              >
                Sign Out
              </Button>
            </div>
          </div>
        </Card>
      </div>

      <OAuthAppSetupModal
        isOpen={showWhoopSetup}
        onClose={() => setShowWhoopSetup(false)}
        onSaved={() => {
          setShowWhoopSetup(false);
          void launchOAuth('whoop');
        }}
        provider="whoop"
        displayName="WHOOP"
        devPortalUrl="https://developer.whoop.com/"
      />
    </div>
  );
}
