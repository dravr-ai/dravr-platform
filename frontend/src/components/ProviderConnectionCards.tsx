// ABOUTME: Provider connection cards for the chat interface empty state
// ABOUTME: Displays fitness providers from server with connection status and OAuth initiation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { providersApi, oauthApi } from '../services/api';
import type { ProviderStatus } from '../services/api';
import { Card, Badge } from './ui';
import { QUERY_KEYS } from '../constants/queryKeys';
import SciotteLoginModal from './SciotteLoginModal';

// Brand colors and hover colors for known providers. After the 2026-Q2 provider
// cleanup the API surfaces only three: `sciotte` (Strava-branded), `sciotte_garmin`
// (Garmin-branded), and `whoop`. Unknown ids fall back to DEFAULT_STYLE below.
const PROVIDER_STYLES: Record<string, { brandColor: string; hoverColor: string }> = {
  sciotte: {
    brandColor: 'bg-[#FC4C02]',
    hoverColor: 'hover:border-[#FC4C02]',
  },
  sciotte_garmin: {
    brandColor: 'bg-[#007CC3]',
    hoverColor: 'hover:border-[#007CC3]',
  },
  whoop: {
    brandColor: 'bg-[#1A1A1A]',
    hoverColor: 'hover:border-[#1A1A1A]',
  },
};

// Default style for unknown providers
const DEFAULT_STYLE = {
  brandColor: 'bg-surface-container-low0',
  hoverColor: 'hover:border-pierre-gray-500',
};

// Get description based on capabilities
const getProviderDescription = (provider: ProviderStatus): string => {
  const caps = provider.capabilities;
  if (caps.includes('activities') && caps.includes('sleep')) {
    return 'Activities, sleep & recovery';
  }
  if (caps.includes('activities')) {
    return 'Activities & workouts';
  }
  if (caps.includes('sleep')) {
    return 'Sleep tracking';
  }
  return 'Fitness data';
};

// SVG icons for each provider - clean and professional. `sciotte` reuses the
// Strava chevron (it's the Strava data path); `sciotte_garmin` reuses the Garmin
// dial. Default falls back to a neutral disc.
const ProviderIcon = ({ providerId, className }: { providerId: string; className?: string }) => {
  const baseClass = className || 'w-5 h-5';

  switch (providerId) {
    case 'sciotte':
      return (
        <svg className={baseClass} viewBox="0 0 24 24" fill="currentColor">
          <path d="M15.387 17.944l-2.089-4.116h-3.065L15.387 24l5.15-10.172h-3.066m-7.008-5.599l2.836 5.598h4.172L10.463 0l-7 13.828h4.169" />
        </svg>
      );
    case 'sciotte_garmin':
      return (
        <svg className={baseClass} viewBox="0 0 24 24" fill="currentColor">
          <path d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm0 18c-4.41 0-8-3.59-8-8s3.59-8 8-8 8 3.59 8 8-3.59 8-8 8zm-1-13h2v6h-2zm0 8h2v2h-2z" />
        </svg>
      );
    case 'whoop':
      return (
        <svg className={baseClass} viewBox="0 0 24 24" fill="currentColor">
          <path d="M12 4C7.58 4 4 7.58 4 12s3.58 8 8 8 8-3.58 8-8-3.58-8-8-8zm0 14c-3.31 0-6-2.69-6-6s2.69-6 6-6 6 2.69 6 6-2.69 6-6 6z" />
          <circle cx="12" cy="12" r="3" />
        </svg>
      );
    default:
      return (
        <svg className={baseClass} viewBox="0 0 24 24" fill="currentColor">
          <path d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm0 18c-4.41 0-8-3.59-8-8s3.59-8 8-8 8 3.59 8 8-3.59 8-8 8z" />
        </svg>
      );
  }
};

interface ProviderConnectionCardsProps {
  onProviderConnected?: () => void;
  onConnectProvider?: (providerName: string) => void;
  connectingProvider?: string | null;
  onSkip?: () => void;
  isSkipPending?: boolean;
  /** Forwarded from `SciotteLoginModal` when the BYO Strava OAuth popup opens. */
  onOAuthLaunched?: (provider: string) => void;
}

export default function ProviderConnectionCards({
  onProviderConnected,
  onConnectProvider,
  connectingProvider,
  onSkip,
  isSkipPending,
  onOAuthLaunched,
}: ProviderConnectionCardsProps) {
  const [sciotteModalTarget, setSciotteModalTarget] = useState<'strava' | 'garmin' | null>(null);
  const queryClient = useQueryClient();

  // Fetch providers from server (includes OAuth and non-OAuth providers)
  const { data: providersData, isLoading, refetch } = useQuery({
    queryKey: QUERY_KEYS.providers.status(),
    queryFn: () => providersApi.getProvidersStatus(),
    refetchInterval: 5000,
  });

  // Handle provider card click
  const handleConnect = async (provider: ProviderStatus) => {
    // If already connected, no action needed
    if (provider.connected) return;

    // Sciotte providers use credential-based login (not OAuth)
    if (provider.provider.startsWith('sciotte')) {
      setSciotteModalTarget(provider.provider === 'sciotte_garmin' ? 'garmin' : 'strava');
      return;
    }

    // Non-OAuth providers (like synthetic) skip directly to chat
    if (!provider.requires_oauth) {
      if (onSkip) onSkip();
      return;
    }

    // Use callback if provided (for chat-based connection flow)
    if (onConnectProvider) {
      onConnectProvider(provider.provider);
      return;
    }

    // Fallback: Navigate directly to OAuth authorization endpoint.
    // Pre-open a blank window synchronously so mobile Safari preserves the
    // user gesture across the authorize-URL await; otherwise the popup is
    // silently blocked and the card sits in a stuck loading state.
    const popup = window.open('about:blank', '_blank');
    try {
      const authUrl = await oauthApi.getAuthorizeUrlForProvider(provider.provider);
      if (popup && !popup.closed) {
        popup.location.href = authUrl;
      } else {
        window.location.href = authUrl;
      }
    } catch (error) {
      if (popup && !popup.closed) {
        popup.close();
      }
      console.error('Failed to get OAuth authorization URL:', error);
    }
  };

  // Check if any provider is connected
  const hasAnyConnection = providersData?.providers?.some(p => p.connected) ?? false;

  // Notify parent when a connection is detected
  if (hasAnyConnection && onProviderConnected) {
    onProviderConnected();
  }

  if (isLoading) {
    return (
      <div className="w-full space-y-2">
        {[1, 2, 3, 4, 5].map((i) => (
          <Card key={i} variant="dark" className="px-5 py-4 animate-pulse">
            <div className="flex items-center gap-4">
              <div className="w-12 h-12 rounded-xl bg-surface-container-high flex-shrink-0" />
              <div className="flex-1">
                <div className="h-4 w-32 bg-surface-container-high rounded mb-2" />
                <div className="h-3 w-48 bg-surface-container-low rounded" />
              </div>
              <div className="h-4 w-16 bg-surface-container-low rounded flex-shrink-0" />
            </div>
          </Card>
        ))}
      </div>
    );
  }

  // `strava` (official OAuth) is reached exclusively through the Sciotte
  // modal's "Use my own Strava OAuth app" button, so don't render a second
  // duplicate card here. Once the user connects via that path, the
  // `provider_connections` row is `strava` and the `connected` badge appears
  // on the Sciotte card via the merge below.
  const stravaOAuthConnection = providersData?.providers?.find((p) => p.provider === 'strava' && p.connected);
  const providers = (providersData?.providers ?? [])
    .filter((p) => p.provider !== 'strava')
    .map((p) =>
      p.provider === 'sciotte' && stravaOAuthConnection && !p.connected
        ? { ...p, connected: true }
        : p,
    );

  return (
    <div className="w-full space-y-2">
      {providers.map((provider) => {
        const style = PROVIDER_STYLES[provider.provider] ?? DEFAULT_STYLE;
        const isConnecting = connectingProvider === provider.provider;
        const isNonOAuth = !provider.requires_oauth && !provider.provider.startsWith('sciotte');
        const isActionable = !provider.connected && (provider.requires_oauth || provider.provider.startsWith('sciotte'));

        return (
          <button
            key={provider.provider}
            type="button"
            onClick={() => handleConnect(provider)}
            disabled={provider.connected || isConnecting || !!connectingProvider}
            className="w-full text-left focus:outline-none focus:ring-2 focus:ring-pierre-violet/50 rounded-xl disabled:cursor-default group"
            aria-label={
              provider.connected
                ? `${provider.display_name} is connected`
                : isNonOAuth
                  ? `${provider.display_name} - ${getProviderDescription(provider)}`
                  : `Connect to ${provider.display_name}`
            }
          >
            <Card
              variant="dark"
              className={`px-5 py-4 transition-all duration-200 border ${
                provider.connected
                  ? 'border-emerald-500/40'
                  : isConnecting
                    ? 'border-primary'
                    : isNonOAuth
                      ? 'border-outline-variant/20 opacity-60'
                      : `border-outline-variant/30 ${style.hoverColor} hover:shadow-md`
              }`}
            >
              <div className="flex items-center gap-4">
                <div
                  className={`flex-shrink-0 w-12 h-12 rounded-xl ${style.brandColor} flex items-center justify-center text-on-surface shadow-sm`}
                >
                  {isConnecting ? (
                    <div className="pierre-spinner w-6 h-6 border-white border-t-transparent"></div>
                  ) : (
                    <ProviderIcon providerId={provider.provider} className="w-6 h-6" />
                  )}
                </div>
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2 flex-wrap">
                    <span className="font-semibold text-on-surface text-base leading-tight">{provider.display_name}</span>
                    {provider.connected && <Badge variant="success">Connected</Badge>}
                    {isNonOAuth && !provider.connected && <Badge variant="secondary">Demo</Badge>}
                  </div>
                  <p className="text-sm text-on-surface-variant mt-0.5 leading-snug">
                    {getProviderDescription(provider)}
                  </p>
                </div>
                {isActionable && (
                  <span className="flex-shrink-0 hidden sm:inline-flex items-center gap-1.5 text-sm font-medium text-on-surface-variant group-hover:text-on-surface transition-colors">
                    Connect
                    <svg
                      className="w-4 h-4"
                      fill="none"
                      stroke="currentColor"
                      viewBox="0 0 24 24"
                    >
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
                    </svg>
                  </span>
                )}
                {isActionable && (
                  <svg
                    className="flex-shrink-0 sm:hidden w-4 h-4 text-outline group-hover:text-on-surface transition-colors"
                    fill="none"
                    stroke="currentColor"
                    viewBox="0 0 24 24"
                  >
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
                  </svg>
                )}
              </div>
            </Card>
          </button>
        );
      })}

      {/* Skip and start chatting - last row */}
      {onSkip && (
        <button
          type="button"
          onClick={onSkip}
          disabled={isSkipPending}
          className="w-full text-left focus:outline-none focus:ring-2 focus:ring-pierre-violet/50 rounded-xl group"
          aria-label="Skip and start chatting"
        >
          <Card
            variant="dark"
            className="px-5 py-4 transition-all duration-200 border border-outline-variant/30 hover:border-primary hover:shadow-md"
          >
            <div className="flex items-center gap-4">
              <div className="flex-shrink-0 w-12 h-12 rounded-xl boreal-hero-gradient flex items-center justify-center text-on-primary shadow-sm">
                {isSkipPending ? (
                  <div className="pierre-spinner w-6 h-6 border-white border-t-transparent"></div>
                ) : (
                  <svg className="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z" />
                  </svg>
                )}
              </div>
              <div className="flex-1 min-w-0">
                <span className="font-semibold text-on-surface text-base leading-tight">
                  {isSkipPending ? 'Starting…' : 'Start chatting'}
                </span>
                <p className="text-sm text-on-surface-variant mt-0.5 leading-snug">
                  Connect providers later
                </p>
              </div>
              <span className="flex-shrink-0 hidden sm:inline-flex items-center gap-1.5 text-sm font-medium text-on-surface-variant group-hover:text-on-surface transition-colors">
                Skip
                <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
                </svg>
              </span>
              <svg
                className="flex-shrink-0 sm:hidden w-4 h-4 text-outline group-hover:text-primary transition-colors"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24"
              >
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
              </svg>
            </div>
          </Card>
        </button>
      )}

      {/* Sciotte login modal */}
      <SciotteLoginModal
        isOpen={sciotteModalTarget !== null}
        onClose={() => setSciotteModalTarget(null)}
        onOAuthLaunched={onOAuthLaunched}
        onConnected={() => {
          refetch();
          // Sciotte completes in-process (no OAuth callback URL), so we have to
          // explicitly bust the onboarding-status cache here. Without this, the
          // App-level route guard stays on OnboardingConnectProvider until the
          // next poll tick (5s), stranding the user on the connected card.
          queryClient.invalidateQueries({ queryKey: QUERY_KEYS.user.onboardingStatus() });
          setSciotteModalTarget(null);
          if (onProviderConnected) onProviderConnected();
        }}
        target={sciotteModalTarget ?? 'strava'}
      />
    </div>
  );
}
