// ABOUTME: Provider connection rows for onboarding — one hairline row per provider: its glyph in its scheme's ink, name, one line, status, action
// ABOUTME: Displays fitness providers from server with connection status and OAuth initiation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState, useEffect } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { providersApi, oauthApi } from '../services/api';
import type { ProviderStatus } from '../services/api';
import { track } from '../services/analytics';
import { QUERY_KEYS } from '../constants/queryKeys';
import { PROVIDER_LINK_POLL_INTERVAL_MS, providerGlyphInk, sciotteTargetForBackend } from '@pierre/shared-constants';
import type { SciotteTarget } from '@pierre/shared-types';
import SciotteLoginModal from './SciotteLoginModal';
import IntervalsIcuLinkModal from './IntervalsIcuLinkModal';
import { useTranslation } from '@pierre/i18n';
import { useTheme } from '../hooks/useTheme';


// One row, whichever provider: a 24px glyph, the name with its one line
// beside it, the status or action on the right, a faint hairline above.
const ROW_CLASS =
  'group flex w-full items-center gap-3 rounded-lg border-t ghost-border-faint py-3 text-left first:border-t-0 focus:outline-none focus-visible:ring-2 focus-visible:ring-primary/50';

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
// dial; `sciotte_trainingpeaks` is a pair of peaks; `sciotte_coros` is COROS'
// official mark (coros.com/public/images/COROS.svg) at its own 1024 scale.
// Default falls back to a neutral disc.
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
    case 'sciotte_trainingpeaks':
      return (
        <svg className={baseClass} viewBox="0 0 24 24" fill="currentColor">
          <path d="M2 20L9 7l4 7 3-5 6 11H2z" />
        </svg>
      );
    case 'sciotte_coros':
      return (
        <svg className={baseClass} viewBox="0 0 1024 1024" fill="currentColor">
          <path d="M611.28637781 226.3848448l313.2594324 182.00737337L925.07539342 786.3244288 612.34554539 967.44210091l-52.8312832-28.51279417 245.1761334-182.36749028L804.22436181 437.85826304 562.20454798 254.81290525l49.08182983-28.42806045zM171.15984213 335.14018133l34.86779961 304.15059058 275.38359524 158.95988452 279.04831715-118.71151332v56.85612089l-313.7678336 181.11767325L120.10795918 728.9599067V366.78811193l51.03069867-31.62674745zM569.19505465 56.55789909l312.72984804 181.11767211 1.80058566 60.13954162-280.04393414-121.80428345-274.9175626 159.76485205-37.02850219 301.75687111-49.06064668-28.42806044 0.50840121-363.12504548L569.19505465 56.55789909z" />
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
}

export default function ProviderConnectionCards({
  onProviderConnected,
  onConnectProvider,
  connectingProvider,
  onSkip,
  isSkipPending,
  onOAuthLaunched,
}: ProviderConnectionCardsProps) {
  const { t } = useTranslation();
  const { scheme } = useTheme();
  const [sciotteModalTarget, setSciotteModalTarget] = useState<SciotteTarget | null>(null);
  // Whether the chosen card still needs its exposure notice accepted.
  const [sciotteConsentRequired, setSciotteConsentRequired] = useState(false);
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

  // Launch the OAuth authorization flow for a provider. Prefers the parent's
  // callback (onboarding shows an "awaiting consent" overlay); otherwise opens
  // the server's launch route, which 302s to the provider. Opening a real
  // same-origin URL keeps the window inside Safari's user-gesture stack without
  // the old `about:blank`-then-assign dance, which left the popup empty for the
  // whole authorize-URL round trip.
  const connectViaOAuth = (providerName: string) => {
    track({ name: 'feature_engaged', props: { feature: 'provider_connect_started' } });
    if (onConnectProvider) {
      onConnectProvider(providerName);
      return;
    }
    const url = oauthApi.authorizeUrl(providerName);
    if (!window.open(url, '_blank')) {
      // Popup blocked outright — same-tab navigation is the documented
      // fallback; the callback records the result either way.
      window.location.href = url;
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

    // Every other scrape-mirror card (Garmin, TrainingPeaks) signs in with the
    // provider's own credentials, after its notice when it has one.
    const target = sciotteTargetForBackend(provider.provider);
    if (target) {
      setSciotteConsentRequired(provider.consent_required);
      setSciotteModalTarget(target);
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
      <div className="w-full">
        {[1, 2, 3, 4, 5].map((i) => (
          <div key={i} className="flex animate-pulse items-center gap-3 border-t ghost-border-faint py-3 first:border-t-0">
            <div className="h-6 w-6 flex-shrink-0 rounded bg-surface-container-high" />
            <div className="h-3 w-40 rounded bg-surface-container-high" />
            <div className="ml-auto h-3 w-16 flex-shrink-0 rounded bg-surface-container-low" />
          </div>
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
    <div className="w-full">
      {providers.map((provider) => {
        // The brand colour is carried by the glyph, not by a tile (DESIGN.md
        // §5), and only in a scheme where it clears the 3:1 icon floor on the
        // canvas; elsewhere — and for a provider with no brand colour — the
        // glyph takes the body ink. `PROVIDER_GLYPH_INK` holds both halves.
        const glyphInk = providerGlyphInk(provider.provider, scheme);
        const isConnecting = connectingProvider === provider.provider;
        const isNonOAuth = !provider.requires_oauth && !provider.provider.startsWith('sciotte') && provider.provider !== 'intervals_icu';
        const isActionable = !provider.connected && (provider.requires_oauth || provider.provider.startsWith('sciotte') || provider.provider === 'intervals_icu');

        return (
          <button
            key={provider.provider}
            type="button"
            onClick={() => handleConnect(provider)}
            disabled={provider.connected || isConnecting || !!connectingProvider}
            className={`${ROW_CLASS} disabled:cursor-default`}
            aria-label={
              provider.connected
                ? t('providers.isConnectedAria', { provider: provider.display_name })
                : isNonOAuth
                  ? `${provider.display_name} - ${t(providerDescriptionKey(provider))}`
                  : t('providers.connectToAria', { provider: provider.display_name })
            }
          >
            <span
              aria-hidden="true"
              data-testid={`provider-glyph-${provider.provider}`}
              className={`flex h-6 w-6 flex-shrink-0 items-center justify-center text-on-surface ${isNonOAuth ? 'opacity-60' : ''}`}
              style={glyphInk ? { color: glyphInk } : undefined}
            >
              {isConnecting ? (
                <div className="pierre-spinner h-5 w-5"></div>
              ) : (
                <ProviderIcon providerId={provider.provider} className="h-5 w-5" />
              )}
            </span>
            <span className={`flex min-w-0 flex-1 flex-wrap items-baseline gap-x-2 ${isNonOAuth ? 'opacity-60' : ''}`}>
              <span className="text-sm font-medium text-on-surface">{provider.display_name}</span>
              <span className="min-w-0 truncate text-xs text-on-surface-variant">{t(providerDescriptionKey(provider))}</span>
            </span>
            {provider.connected && (
              <span className="inline-flex flex-shrink-0 items-center gap-1.5 text-xs text-on-surface-variant">
                <span aria-hidden="true" className="h-1.5 w-1.5 rounded-full bg-success" />
                {t('providers.connected')}
              </span>
            )}
            {isNonOAuth && !provider.connected && (
              <span className="flex-shrink-0 text-xs text-outline">{t('providers.demoBadge')}</span>
            )}
            {isActionable && (
              <span className="flex-shrink-0 text-sm font-medium text-primary transition-colors group-hover:text-primary-hover">
                {t('providers.connect')}
              </span>
            )}
          </button>
        );
      })}

      {/* Skip and start chatting - last row */}
      {onSkip && (
        <button
          type="button"
          onClick={onSkip}
          disabled={isSkipPending}
          className={ROW_CLASS}
          aria-label={t('providers.skipAndChat')}
        >
          <span aria-hidden="true" className="flex h-6 w-6 flex-shrink-0 items-center justify-center text-primary">
            {isSkipPending ? (
              <div className="pierre-spinner h-5 w-5"></div>
            ) : (
              <svg className="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z" />
              </svg>
            )}
          </span>
          <span className="flex min-w-0 flex-1 flex-wrap items-baseline gap-x-2">
            <span className="text-sm font-medium text-on-surface">
              {isSkipPending ? t('providers.starting') : t('providers.startChatting')}
            </span>
            <span className="min-w-0 truncate text-xs text-on-surface-variant">{t('providers.connectLater')}</span>
          </span>
          <span className="flex-shrink-0 text-sm font-medium text-primary transition-colors group-hover:text-primary-hover">
            {t('common.skip')}
          </span>
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
        consentRequired={sciotteConsentRequired}
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
