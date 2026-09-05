// ABOUTME: Provider connection cards for the chat interface empty state
// ABOUTME: Displays fitness providers from server with connection status and OAuth initiation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState, useEffect } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { providersApi, oauthApi } from '../services/api';
import type { ProviderStatus } from '../services/api';
import { track } from '../services/analytics';
import { Card, Badge } from './ui';
import { QUERY_KEYS } from '../constants/queryKeys';
import { PROVIDER_LINK_POLL_INTERVAL_MS } from '@pierre/shared-constants';
import SciotteLoginModal from './SciotteLoginModal';
import IntervalsIcuLinkModal from './IntervalsIcuLinkModal';
import { useTranslation } from '@pierre/i18n';

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
  intervals_icu: {
    brandColor: 'bg-[#1273DE]',
    hoverColor: 'hover:border-[#1273DE]',
  },
};

// Default style for unknown providers
const DEFAULT_STYLE = {
  brandColor: 'bg-surface-container-low',
  hoverColor: 'hover:border-on-surface-variant',
};

// The corpus key of the one-line blurb a provider's capability set earns;
// the card resolves it with t() so the line reads in the athlete's language.
const providerDescriptionKey = (provider: ProviderStatus): string => {
  const caps = provider.capabilities;
  if (caps.includes('activities') && caps.includes('sleep')) {
    return 'providerBlurb.activitiesSleepRecovery';
  }
  if (caps.includes('activities')) {
    return 'providerBlurb.activitiesWorkouts';
  }
  if (caps.includes('sleep')) {
    return 'providerBlurb.sleepTracking';
  }
  return 'providerBlurb.fitnessData';
};

// SVG icons for each provider - clean and professional. `sciotte` reuses the
// Strava chevron (it's the Strava data path); `sciotte_garmin` reuses the Garmin
// dial. Default falls back to a neutral disc.
export const ProviderIcon = ({ providerId, className }: { providerId: string; className?: string }) => {
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
    case 'intervals_icu':
      return (
        <svg className={baseClass} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
          <path d="M3 13h4l3 7 4-14 3 7h4" />
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
  /**
   * Bumped by the onboarding parent when its delegated Strava OAuth *launch*
   * fails (authorize-URL fetch throws) so this component — which owns the
   * Sciotte modal — opens the credential-login fallback. Post-consent OAuth
   * failures are caught separately via the `pierre_oauth_result` listener.
   */
  oauthLaunchFailedNonce?: number;
}

export default function ProviderConnectionCards({
  onProviderConnected,
  onConnectProvider,
  connectingProvider,
  onSkip,
  isSkipPending,
  onOAuthLaunched,
  oauthLaunchFailedNonce,
}: ProviderConnectionCardsProps) {
  const { t } = useTranslation();
  const [sciotteModalTarget, setSciotteModalTarget] = useState<'strava' | 'garmin' | null>(null);
  const [intervalsModalOpen, setIntervalsModalOpen] = useState(false);
  const queryClient = useQueryClient();

  // Fetch providers from server (includes OAuth and non-OAuth providers).
  //
  // The 5s poll is transient by intent: an OAuth grant completes in a *second
  // tab*, so this one can only learn about it by asking. It therefore runs
  // while there is an answer to wait for — an attempt in flight, or no
  // connection landed yet — and stops the moment one arrives, rather than
  // ticking for as long as the screen is mounted. A second provider connected
  // later re-arms it through `connectingProvider`.
  const { data: providersData, isLoading, refetch } = useQuery({
    queryKey: QUERY_KEYS.providers.status(),
    queryFn: () => providersApi.getProvidersStatus(),
    refetchInterval: query => {
      if (connectingProvider) return PROVIDER_LINK_POLL_INTERVAL_MS;
      const landed = query.state.data?.providers?.some(p => p.connected) ?? false;
      return landed ? false : PROVIDER_LINK_POLL_INTERVAL_MS;
    },
  });

  // OAuth-first with a Sciotte fallback: the `sciotte` card launches real Strava
  // OAuth while shared-app seats remain. If that OAuth attempt fails — the
  // athlete cap was actually exceeded in a seat-count race, or the provider
  // rejected the grant — we don't strand the user. The OAuth callback tab writes
  // `pierre_oauth_result` (firing a `storage` event here in the opener); on a
  // failed Strava result we open the Sciotte credential login for the same data.
  // Only *failed* Strava results are consumed — successful results are left
  // untouched for the success handlers in ChatTab and UserSettings.
  useEffect(() => {
    const consumeFailedStrava = () => {
      let stored: string | null;
      try {
        stored = localStorage.getItem('pierre_oauth_result');
      } catch {
        return;
      }
      if (!stored) return;
      try {
        const result = JSON.parse(stored);
        const fresh = result?.timestamp && Date.now() - result.timestamp < 30_000;
        if (fresh && result.provider === 'strava' && result.success === false) {
          localStorage.removeItem('pierre_oauth_result');
          setSciotteModalTarget('strava');
        }
      } catch {
        // Ignore parse errors — leave the entry for other consumers.
      }
    };
    const onStorage = (e: StorageEvent) => {
      if (e.key === 'pierre_oauth_result' && e.newValue) consumeFailedStrava();
    };
    window.addEventListener('storage', onStorage);
    window.addEventListener('focus', consumeFailedStrava);
    document.addEventListener('visibilitychange', consumeFailedStrava);
    consumeFailedStrava();
    return () => {
      window.removeEventListener('storage', onStorage);
      window.removeEventListener('focus', consumeFailedStrava);
      document.removeEventListener('visibilitychange', consumeFailedStrava);
    };
  }, []);

  // Onboarding delegates the Strava OAuth *launch* to the parent
  // (OnboardingConnectProvider). If the parent's authorize-URL fetch throws, it
  // bumps `oauthLaunchFailedNonce` so we open the Sciotte fallback here, where
  // the modal lives.
  useEffect(() => {
    if (oauthLaunchFailedNonce && oauthLaunchFailedNonce > 0) {
      setSciotteModalTarget('strava');
    }
  }, [oauthLaunchFailedNonce]);

  // Launch the OAuth authorization flow for a provider. Prefers the parent's
  // callback (onboarding shows an "awaiting consent" overlay); otherwise opens
  // the authorize URL directly. Pre-opens a blank window synchronously so
  // mobile Safari preserves the user gesture across the authorize-URL await;
  // otherwise the popup is silently blocked and the card sits in a stuck state.
  const connectViaOAuth = async (providerName: string) => {
    track({ name: 'feature_engaged', props: { feature: 'provider_connect_started' } });
    if (onConnectProvider) {
      onConnectProvider(providerName);
      return;
    }
    const popup = window.open('about:blank', '_blank');
    try {
      const authUrl = await oauthApi.getAuthorizeUrlForProvider(providerName);
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
      // Couldn't start OAuth (network/config error). For the Strava card, fall
      // back to the Sciotte credential login instead of leaving the card stuck.
      if (providerName === 'strava') {
        setSciotteModalTarget('strava');
      }
    }
  };

  // Handle provider card click
  const handleConnect = async (provider: ProviderStatus) => {
    // If already connected, no action needed
    if (provider.connected) return;

    // The Sciotte card is the user-facing "Strava" card. While shared-app OAuth
    // seats remain the server recommends `oauth`, so connect via the official
    // Strava OAuth flow. Once the athlete cap is reached the server recommends
    // `mirror`, and we silently fall back to the Sciotte credential login — the
    // user taps the same "Connect Strava" card either way.
    if (provider.provider === 'sciotte') {
      if (provider.recommended_backend === 'oauth') {
        await connectViaOAuth('strava');
      } else {
        setSciotteModalTarget('strava');
      }
      return;
    }

    // Other Sciotte backends (Garmin) always use credential-based login.
    if (provider.provider.startsWith('sciotte')) {
      setSciotteModalTarget('garmin');
      return;
    }

    // Intervals.icu is an API-key provider — open the athlete-id + key modal.
    if (provider.provider === 'intervals_icu') {
      setIntervalsModalOpen(true);
      return;
    }

    // Non-OAuth providers (like synthetic) skip directly to chat
    if (!provider.requires_oauth) {
      if (onSkip) onSkip();
      return;
    }

    await connectViaOAuth(provider.provider);
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
  // duplicate card here. Connecting by that path writes a `strava`
  // `provider_connections` row, and the Sciotte card still reads as connected
  // because the server coalesces a card's two backends before answering
  // (carnet#255) — this used to be merged here, and in the mobile client, and
  // neither copy covered Garmin.
  const providers = (providersData?.providers ?? [])
    // Hide the raw `strava` OAuth (the `sciotte` card is the Strava data path) and
    // `garmin` ("Garmin Connect") — Garmin's OAuth API is uncredentialed/unsupported,
    // so it must not be offered. The `sciotte_garmin` ("Garmin") scrape card stays.
    .filter((p) => p.provider !== 'strava' && p.provider !== 'garmin');

  return (
    <div className="w-full space-y-2">
      {providers.map((provider) => {
        const style = PROVIDER_STYLES[provider.provider] ?? DEFAULT_STYLE;
        const isConnecting = connectingProvider === provider.provider;
        const isNonOAuth = !provider.requires_oauth && !provider.provider.startsWith('sciotte') && provider.provider !== 'intervals_icu';
        const isActionable = !provider.connected && (provider.requires_oauth || provider.provider.startsWith('sciotte') || provider.provider === 'intervals_icu');

        return (
          <button
            key={provider.provider}
            type="button"
            onClick={() => handleConnect(provider)}
            disabled={provider.connected || isConnecting || !!connectingProvider}
            className="w-full text-left focus:outline-none focus:ring-2 focus:ring-primary/50 rounded-xl disabled:cursor-default group"
            aria-label={
              provider.connected
                ? t('providers.isConnectedAria', { provider: provider.display_name })
                : isNonOAuth
                  ? `${provider.display_name} - ${t(providerDescriptionKey(provider))}`
                  : t('providers.connectToAria', { provider: provider.display_name })
            }
          >
            <Card
              variant="dark"
              className={`px-5 py-4 transition-all duration-200 border ${
                provider.connected
                  ? 'border-success/40'
                  : isConnecting
                    ? 'border-primary'
                    : isNonOAuth
                      ? 'border-outline-variant/20 opacity-60'
                      : `border-outline-variant/30 ${style.hoverColor}`
              }`}
            >
              <div className="flex items-center gap-4">
                <div
                  className={`flex-shrink-0 w-12 h-12 rounded-xl ${style.brandColor} flex items-center justify-center text-on-surface`}
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
                    {provider.connected && <Badge variant="success">{t('providers.connected')}</Badge>}
                    {isNonOAuth && !provider.connected && <Badge variant="secondary">{t('providers.demoBadge')}</Badge>}
                  </div>
                  <p className="text-sm text-on-surface-variant mt-0.5 leading-snug">
                    {t(providerDescriptionKey(provider))}
                  </p>
                </div>
                {isActionable && (
                  <span className="flex-shrink-0 hidden sm:inline-flex items-center gap-1.5 text-sm font-medium text-on-surface-variant group-hover:text-on-surface transition-colors">
                    {t('providers.connect')}
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
          className="w-full text-left focus:outline-none focus:ring-2 focus:ring-primary/50 rounded-xl group"
          aria-label={t('providers.skipAndChat')}
        >
          <Card
            variant="dark"
            className="px-5 py-4 transition-all duration-200 border border-outline-variant/30 hover:border-primary"
          >
            <div className="flex items-center gap-4">
              <div className="flex-shrink-0 w-12 h-12 rounded-xl bg-primary-container flex items-center justify-center text-on-primary-container">
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
                  {isSkipPending ? t('providers.starting') : t('providers.startChatting')}
                </span>
                <p className="text-sm text-on-surface-variant mt-0.5 leading-snug">
                  {t('providers.connectLater')}
                </p>
              </div>
              <span className="flex-shrink-0 hidden sm:inline-flex items-center gap-1.5 text-sm font-medium text-on-surface-variant group-hover:text-on-surface transition-colors">
                {t('common.skip')}
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

      {/* Intervals.icu API-key link modal */}
      <IntervalsIcuLinkModal
        isOpen={intervalsModalOpen}
        onClose={() => setIntervalsModalOpen(false)}
        onConnected={() => {
          refetch();
          // Intervals.icu connects in-process (no OAuth callback), so bust the
          // onboarding-status cache explicitly — same reasoning as Sciotte above.
          queryClient.invalidateQueries({ queryKey: QUERY_KEYS.user.onboardingStatus() });
          setIntervalsModalOpen(false);
          if (onProviderConnected) onProviderConnected();
        }}
      />
    </div>
  );
}
