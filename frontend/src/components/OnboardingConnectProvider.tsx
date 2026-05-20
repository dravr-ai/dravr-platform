// ABOUTME: First-run onboarding screen that forces a provider connection before the user reaches the dashboard
// ABOUTME: Reuses ProviderConnectionCards without onSkip so there's no escape hatch; OAuth completion flips the gate

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import { oauthApi } from '../services/api';
import ProviderConnectionCards from './ProviderConnectionCards';

/**
 * Hard gate shown right after first login when the user has zero connected
 * providers. The same source of truth — `provider_connections` — drives the
 * backend's `NoProviderConnected` 403 on chat/coach/messaging, so this screen
 * cannot drift from the server-side enforcement. Once the user connects any
 * provider, the OAuth callback in `App.tsx` invalidates the onboarding-status
 * query and the redirect flips to the dashboard.
 *
 * Skip button is intentionally absent: the LLM coach has nothing to reason
 * about without provider data, and letting the user past this screen would
 * land them on a chat that hallucinates specifics from their message.
 */
export default function OnboardingConnectProvider({
  userDisplayName,
}: {
  userDisplayName?: string | null;
}) {
  const [connectingProvider, setConnectingProvider] = useState<string | null>(null);

  const handleConnectProvider = async (provider: string) => {
    setConnectingProvider(provider);
    try {
      const authUrl = await oauthApi.getAuthorizeUrlForProvider(provider);
      window.open(authUrl, '_blank');
    } catch (error) {
      console.error(`Failed to get OAuth URL for ${provider}:`, error);
      setConnectingProvider(null);
    }
  };

  return (
    <div className="min-h-screen bg-surface flex items-center justify-center px-4 py-12">
      <div className="max-w-2xl w-full">
        <div className="text-center mb-8">
          <h1 className="text-3xl font-bold text-on-surface mb-3">
            {userDisplayName ? `Welcome, ${userDisplayName}` : 'Welcome to Dravr'}
          </h1>
          <p className="text-base text-on-surface-variant max-w-lg mx-auto">
            Connect a fitness service to get started. Dravr coaches you on the activities your
            provider already tracks — without one, there's nothing for the model to read.
          </p>
        </div>

        <ProviderConnectionCards
          onConnectProvider={handleConnectProvider}
          connectingProvider={connectingProvider}
        />

        <p className="text-xs text-on-surface-variant text-center mt-8">
          We use the OAuth flow from your provider — Dravr never sees your username or password.
        </p>
      </div>
    </div>
  );
}
