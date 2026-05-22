// ABOUTME: First-run onboarding screen that forces a provider connection before the user reaches the dashboard
// ABOUTME: Mirrors the PendingApproval system-page pattern (boreal-hero-gradient accent + DravrLogo + Card chrome)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import { oauthApi } from '../services/api';
import { useAuth } from '../hooks/useAuth';
import { Button, Card } from './ui';
import ProviderConnectionCards from './ProviderConnectionCards';

// Dravr boreal-palette logo. Same dot-and-line motif as Login and
// PendingApproval; gradient IDs are suffixed with `-onboarding` so the
// embedded `<defs>` don't collide when multiple instances of the logo render
// in the same document tree.
function DravrLogo() {
  return (
    <svg width="64" height="64" viewBox="0 0 120 120" xmlns="http://www.w3.org/2000/svg">
      <defs>
        <linearGradient id="pg-onboarding" x1="0%" y1="0%" x2="100%" y2="100%">
          <stop offset="0%" style={{ stopColor: '#00241a' }} />
          <stop offset="100%" style={{ stopColor: '#0d3b2e' }} />
        </linearGradient>
        <linearGradient id="ag-onboarding" x1="0%" y1="0%" x2="100%" y2="100%">
          <stop offset="0%" style={{ stopColor: '#3c6658' }} />
          <stop offset="100%" style={{ stopColor: '#234e40' }} />
        </linearGradient>
        <linearGradient id="ng-onboarding" x1="0%" y1="0%" x2="100%" y2="100%">
          <stop offset="0%" style={{ stopColor: '#8f6a2e' }} />
          <stop offset="100%" style={{ stopColor: '#6e5020' }} />
        </linearGradient>
        <linearGradient id="rg-onboarding" x1="0%" y1="0%" x2="100%" y2="100%">
          <stop offset="0%" style={{ stopColor: '#5e7a82' }} />
          <stop offset="100%" style={{ stopColor: '#425962' }} />
        </linearGradient>
      </defs>
      <g strokeWidth="2" opacity="0.55" strokeLinecap="round">
        <line x1="40" y1="30" x2="52" y2="42" stroke="url(#ag-onboarding)" />
        <line x1="52" y1="42" x2="70" y2="35" stroke="url(#ag-onboarding)" />
        <line x1="52" y1="42" x2="48" y2="55" stroke="url(#pg-onboarding)" />
        <line x1="48" y1="55" x2="75" y2="52" stroke="url(#ng-onboarding)" />
        <line x1="48" y1="55" x2="55" y2="72" stroke="url(#pg-onboarding)" />
        <line x1="55" y1="72" x2="35" y2="85" stroke="url(#rg-onboarding)" />
        <line x1="55" y1="72" x2="72" y2="82" stroke="url(#rg-onboarding)" />
      </g>
      <circle cx="40" cy="30" r="7" fill="url(#ag-onboarding)" />
      <circle cx="52" cy="42" r="5" fill="url(#ag-onboarding)" />
      <circle cx="70" cy="35" r="3.5" fill="url(#ag-onboarding)" />
      <circle cx="48" cy="55" r="6" fill="url(#pg-onboarding)" />
      <circle cx="48" cy="55" r="3" fill="#f9f9f6" opacity="0.9" />
      <circle cx="75" cy="52" r="4.5" fill="url(#ng-onboarding)" />
      <circle cx="88" cy="60" r="3.5" fill="url(#ng-onboarding)" />
      <circle cx="55" cy="72" r="5" fill="url(#rg-onboarding)" />
      <circle cx="35" cy="85" r="4" fill="url(#rg-onboarding)" />
      <circle cx="72" cy="82" r="4" fill="url(#rg-onboarding)" />
    </svg>
  );
}

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
}: {
  userDisplayName?: string | null;
}) {
  const { logout } = useAuth();
  const [connectingProvider, setConnectingProvider] = useState<string | null>(null);

  const handleConnectProvider = async (provider: string) => {
    // Mobile Safari requires window.open to fire inside the synchronous
    // user-gesture call stack. Pre-open a blank window so the popup permission
    // is captured before the async authorize-URL fetch.
    const popup = window.open('about:blank', '_blank');
    setConnectingProvider(provider);
    try {
      const authUrl = await oauthApi.getAuthorizeUrlForProvider(provider);
      if (popup && !popup.closed) {
        popup.location.href = authUrl;
      } else {
        // Popup blocked even synchronously — fall back to same-tab navigation.
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
    }
  };

  return (
    <div className="min-h-screen flex items-center justify-center py-12 px-4 sm:px-6 lg:px-8 bg-surface-container-low">
      <div className="max-w-2xl w-full">
        <Card className="overflow-hidden">
          {/* Gradient accent bar — matches the PendingApproval brand moment. */}
          <div className="h-1 w-full boreal-hero-gradient" />

          <div className="px-8 py-10">
            <div className="flex flex-col items-center text-center">
              <DravrLogo />

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
              />
            </div>

            <p className="mt-6 text-xs text-on-surface-variant text-center">
              We use the OAuth flow from your provider — Dravr never sees your
              username or password.
            </p>

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
    </div>
  );
}
