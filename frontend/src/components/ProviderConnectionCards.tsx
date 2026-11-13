// ABOUTME: Provider connection rows for onboarding — one hairline row per provider: its glyph in its scheme's ink, name, one line, status, action
// ABOUTME: Each row follows the card's connect plan: connect, reconnect, the move to the official connection, or the sign-in that copies its history
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState, useEffect, useRef } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { providersApi, oauthApi } from '../services/api';
import type { ProviderStatus } from '../services/api';
import { track } from '../services/analytics';
import { QUERY_KEYS } from '../constants/queryKeys';
import {
  HISTORY_ACTION_KEYS,
  HISTORY_IMPORTING_REFRESH_MS,
  HISTORY_LINE_KEYS,
  PROVIDER_LINK_POLL_INTERVAL_MS,
  cardStatus,
  connectPlan,
  historyActionKey,
  historyImporting,
  noticeRequired,
  oauthFallbackTarget,
  providerGlyphInk,
  refusalReturnsToCard,
  type CardStatus,
} from '@pierre/shared-constants';
import type { SciotteTarget } from '@pierre/shared-types';
import SciotteLoginModal from './SciotteLoginModal';
import IntervalsIcuLinkModal from './IntervalsIcuLinkModal';
import OAuthAppSetupModal from './OAuthAppSetupModal';
import { ownAppDevPortal } from '@pierre/domain-utils';
import { ProviderNoticeDialog } from './ProviderNotice';
import ProviderExportLink from './import/ProviderExportLink';
import ImportOwnDataLink from './import/ImportOwnDataLink';
import { useImportResultRefresh } from './import/useImportResultRefresh';
import {
  OAUTH_RESULT_FRESH_MS,
  OAUTH_RESULT_KEY,
  historySignInAfterGrant,
  refusalAnswer,
  startOAuthInThisTab,
  type OAuthResult,
} from '../utils/oauthReturn';
import { useTranslation } from '@pierre/i18n';
import { useTheme } from '../hooks/useTheme';

// One row, whichever provider: a 24px glyph, the name with its one line
// beside it, the status or action on the right, a faint hairline above.
const ROW_CLASS =
  'group flex w-full items-center gap-3 rounded-lg py-3 text-left focus:outline-none focus-visible:ring-2 focus-visible:ring-primary/50';
// The hairline sits on the row's wrapper, which also holds the card's export link.
const ROW_WRAPPER_CLASS = 'border-t ghost-border-faint first:border-t-0';

/**
 * The account sign-in that copies `card`'s history now, or `null`: its history
 * is still needed or did not finish and the server names the sign-in. With
 * none, the row offers no history action.
 */
function historySignInNow(card: ProviderStatus, status: CardStatus): SciotteTarget | null {
  if (status !== 'history_needed' && status !== 'history_incomplete') return null;
  return connectPlan(card).history.signIn;
}

/** The card states a tap acts on; the others (connected, copying, copied) leave the row inert. */
const ACTIONABLE: ReadonlySet<CardStatus> = new Set<CardStatus>([
  'authorize_owed',
  'reconnect',
  'migrate',
  'history_needed',
  'history_incomplete',
  'connect',
]);

/**
 * The account sign-in on screen, whether it is a card's one-time history
 * copy, and the card whose OAuth a sign-in that cannot complete falls back to
 * (`oauthFallbackTarget`, carnet#1477), when it may.
 */
interface SignInWindow {
  target: SciotteTarget;
  consentRequired: boolean;
  historyMode: boolean;
  fallback: { provider: SciotteTarget; card: ProviderStatus } | null;
}

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
  onSkip?: () => void;
  isSkipPending?: boolean;
  /**
   * Shows only these providers (by `provider` name), in this order.
   * The coach-seat step passes the coaching platforms; every other
   * caller shows the full served list, in the server's order.
   */
  onlyProviders?: readonly string[];
  /**
   * Each row is the glyph, the name and its action, nothing else: no
   * description, no export link, and no import-your-own-data link under the
   * list. Where a coaching platform is one route to the coach seat, beside
   * another, every route reads the same.
   */
  plain?: boolean;
}

export default function ProviderConnectionCards({
  onSkip,
  isSkipPending,
  onlyProviders,
  plain = false,
}: ProviderConnectionCardsProps) {
  const { t } = useTranslation();
  const { scheme } = useTheme();
  const [signIn, setSignIn] = useState<SignInWindow | null>(null);
  // The card whose grant came back refused: its row says so and its tap tries again.
  const [refusedCard, setRefusedCard] = useState<string | null>(null);
  // The Intervals.icu dialog, and whether it leads with the authorization
  // (`intervalsConnectPath` answered `'choose'`) or is the key form alone.
  const [intervalsModal, setIntervalsModal] = useState<{ offersAuthorize: boolean } | null>(null);
  // The OAuth provider whose notice is on screen before its authorization
  // page opens (WHOOP, until the account accepts its owner authorization).
  const [noticeProvider, setNoticeProvider] = useState<string | null>(null);
  // A provider no app of the server's can authorize for this athlete
  // (`own_app_required`) asks for an app of their own first; saving it starts
  // the OAuth flow, carrying the notice acceptance that led here.
  const [ownAppSetup, setOwnAppSetup] = useState<{
    provider: ProviderStatus;
    devPortalUrl: string;
    tosConsent: boolean;
  } | null>(null);
  const queryClient = useQueryClient();
  // The grant (by its result's timestamp) whose history sign-in was already
  // opened, so a later focus does not reopen it while the result stays fresh.
  const chainedGrant = useRef<number | null>(null);
  useImportResultRefresh();

  // Fetch providers from server (includes OAuth and non-OAuth providers).
  //
  // The 5s poll is transient by intent: a connection can land somewhere this
  // screen does not see — the hosted connect page a chat link opens, on
  // another device — so it asks while no connection has landed yet, and stops
  // the moment one arrives rather than ticking for as long as the screen is
  // mounted. A card copying its history is read again until the copy settles.
  // A connect started here returns to this tab, which reloads the status.
  const { data: providersData, isLoading, refetch } = useQuery({
    queryKey: QUERY_KEYS.providers.status(),
    queryFn: () => providersApi.getProvidersStatus(),
    refetchInterval: query => {
      const cards = query.state.data?.providers ?? [];
      if (historyImporting(cards)) return HISTORY_IMPORTING_REFRESH_MS;
      return cards.some(p => p.connected) ? false : PROVIDER_LINK_POLL_INTERVAL_MS;
    },
  });

  // Only the sign-in a card's own plan opens may fall back to its OAuth
  // (`fromPlan`): one opened because the provider refused its grant would
  // bounce the athlete between the two, and a history copy has its grant.
  const openSignIn = (card: ProviderStatus | undefined, target: SciotteTarget, historyMode = false, fromPlan = false) => {
    const oauth = card && fromPlan && !historyMode ? oauthFallbackTarget(card) : null;
    setSignIn({
      target,
      consentRequired: card?.consent_required ?? false,
      historyMode,
      fallback: card && oauth ? { provider: oauth, card } : null,
    });
  };

  // The result a connect started here came back with (`settleOAuthReturn`).
  // A grant reloads the cards and, when the granted card still needs its
  // history, opens the sign-in that copies it, once (carnet#1472). A refused
  // Strava or Garmin grant is answered on its card (`refusalAnswer`): Strava opens
  // its sign-in; Garmin follows its plan as the cards now read — the sign-in
  // when the seats ran out, else "didn't connect, try again" on the row.
  // Only refusals are consumed — a grant's entry is left for the success
  // handlers in useOAuthCompletion and UserSettings.
  useEffect(() => {
    const consumeOAuthResult = () => {
      let result: OAuthResult;
      try {
        const stored = localStorage.getItem(OAUTH_RESULT_KEY);
        if (!stored) return;
        result = JSON.parse(stored);
      } catch {
        return;
      }
      const fresh = result?.timestamp && Date.now() - result.timestamp < OAUTH_RESULT_FRESH_MS;
      if (!fresh) return;
      // The poll stops once any provider is connected, so a second connection
      // is picked up here. The onboarding status is refetched too, so this
      // tab leaves the connect gate and reaches the coach proposal (carnet#1042).
      if (result.success === true) {
        void refetch().then(({ data }) => {
          if (chainedGrant.current === result.timestamp) return;
          const chained = historySignInAfterGrant(result, data?.providers ?? []);
          if (!chained) return;
          chainedGrant.current = result.timestamp;
          openSignIn(chained.card, chained.target, true);
        });
        void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.user.onboardingStatus() });
        return;
      }
      // Only a refused Strava or Garmin grant is this list's to answer; the
      // callback page answered any other.
      if (result.success !== false || result.tombstoned || !refusalReturnsToCard(result.provider)) return;
      void refetch().then(({ data }) => {
        const answer = refusalAnswer(result, data?.providers ?? []);
        if (!answer) return;
        try {
          localStorage.removeItem(OAUTH_RESULT_KEY);
        } catch {
          // Storage refused: the freshness window ends the entry.
        }
        if (answer.kind === 'sign_in') {
          openSignIn(answer.card, answer.target);
        } else {
          setRefusedCard(answer.card.provider);
        }
      });
    };
    const onStorage = (e: StorageEvent) => {
      if (e.key === OAUTH_RESULT_KEY && e.newValue) consumeOAuthResult();
    };
    window.addEventListener('storage', onStorage);
    window.addEventListener('focus', consumeOAuthResult);
    document.addEventListener('visibilitychange', consumeOAuthResult);
    consumeOAuthResult();
    return () => {
      window.removeEventListener('storage', onStorage);
      window.removeEventListener('focus', consumeOAuthResult);
      document.removeEventListener('visibilitychange', consumeOAuthResult);
    };
  }, [refetch, queryClient]);

  // A sign-in that could not complete hands over to the card's OAuth
  // (carnet#1477): the window closes and the authorization starts in its place.
  const fallBackToOAuth =
    ({ provider, card }: NonNullable<SignInWindow['fallback']>) =>
    () => {
      setSignIn(null);
      connectViaOAuth(provider, card);
    };

  // Launch the OAuth authorization flow for a provider: go, in this tab, to
  // the server's launch route, which 302s to the provider and returns here
  // (`startOAuthInThisTab`). `card` is the card tapped, whose own-app setup
  // comes first when no app of the server's can authorize it.
  const connectViaOAuth = (providerName: string, card?: ProviderStatus, tosConsent = false) => {
    const devPortalUrl = ownAppDevPortal(providerName);
    if (card?.own_app_required && devPortalUrl) {
      setOwnAppSetup({ provider: card, devPortalUrl, tosConsent });
      return;
    }
    launchOAuth(providerName, tosConsent);
  };

  const launchOAuth = (providerName: string, tosConsent: boolean) => {
    track({ name: 'feature_engaged', props: { feature: 'provider_connect_started' } });
    startOAuthInThisTab(oauthApi.authorizeUrl(providerName, { tosConsent }));
  };

  // A tap follows what the card shows (`cardStatus`) and how it connects
  // (`connectPlan`): a connected WHOOP that owes its owner authorization asks
  // for the notice; a card still needing its history opens the sign-in that
  // copies it; any card to connect, reconnect or move to its official
  // connection follows its plan's primary way in. Connected cards are inert.
  const handleConnect = (provider: ProviderStatus) => {
    const status = cardStatus(provider);
    if (status === 'authorize_owed') {
      setNoticeProvider(provider.provider);
      return;
    }
    if (status === 'history_needed' || status === 'history_incomplete') {
      const target = historySignInNow(provider, status);
      if (target) openSignIn(provider, target, true);
      return;
    }
    if (!ACTIONABLE.has(status)) return;
    setRefusedCard(null);
    const primary = connectPlan(provider).primary;
    switch (primary.kind) {
      case 'oauth':
        // A provider whose notice the account has not accepted (WHOOP's owner
        // authorization) shows it first; its Continue starts the flow.
        if (primary.provider === provider.provider && noticeRequired(provider.provider, provider.consent_required)) {
          setNoticeProvider(provider.provider);
          return;
        }
        connectViaOAuth(primary.provider, provider);
        return;
      case 'sign_in':
        // The provider's own account sign-in, after its notice when it has one.
        openSignIn(provider, primary.target, false, true);
        return;
      case 'intervals':
        // The authorization first while the server offers it, with the
        // coach's API-key form one tap away; the key form alone otherwise.
        setIntervalsModal({ offersAuthorize: primary.path === 'choose' });
        return;
      case 'none':
        // Non-OAuth providers (like synthetic) skip directly to chat.
        if (onSkip) onSkip();
    }
  };

  if (isLoading) {
    return (
      <div className="w-full">
        {Array.from({ length: onlyProviders?.length ?? 5 }, (_, i) => i).map((i) => (
          <div key={i} className="flex animate-pulse items-center gap-3 border-t ghost-border-faint py-3 first:border-t-0">
            <div className="h-6 w-6 flex-shrink-0 rounded bg-surface-container-high" />
            <div className="h-3 w-40 rounded bg-surface-container-high" />
            <div className="ml-auto h-3 w-16 flex-shrink-0 rounded bg-surface-container-low" />
          </div>
        ))}
      </div>
    );
  }

  // Rendered as served: the server withholds the raw `strava` / `garmin` rows
  // a mirror card covers and coalesces a card's two backends before answering
  // (carnet#255, carnet#574), so no client filters or merges them again. The
  // one narrowing here is by audience (`onlyProviders`), never by backend.
  const served = providersData?.providers ?? [];
  const providers = onlyProviders
    ? served
        .filter((p) => onlyProviders.includes(p.provider))
        .sort((a, b) => onlyProviders.indexOf(a.provider) - onlyProviders.indexOf(b.provider))
    : served;

  return (
    <div className="w-full">
      {providers.map((provider) => {
        // The brand colour is carried by the glyph, not by a tile (DESIGN.md
        // §5), and only in a scheme where it clears the 3:1 icon floor on the
        // canvas; elsewhere — and for a provider with no brand colour — the
        // glyph takes the body ink. `PROVIDER_GLYPH_INK` holds both halves.
        const glyphInk = providerGlyphInk(provider.provider, scheme);
        const status = cardStatus(provider);
        const isNonOAuth = connectPlan(provider).primary.kind === 'none';
        const offersHistory = historySignInNow(provider, status) !== null;
        // A card whose history has no sign-in to copy it is connected and inert.
        const historyWithoutSignIn = (status === 'history_needed' || status === 'history_incomplete') && !offersHistory;
        const actionable = ACTIONABLE.has(status) && !historyWithoutSignIn;
        const refused =
          refusedCard === provider.provider && (status === 'connect' || status === 'reconnect' || status === 'migrate');
        const name = { provider: provider.display_name };
        const historyHintId = `provider-history-hint-${provider.provider}`;

        return (
          <div key={provider.provider} className={ROW_WRAPPER_CLASS} data-testid={`provider-card-${provider.provider}`}>
            <button
              type="button"
              onClick={() => handleConnect(provider)}
              disabled={!actionable}
              className={`${ROW_CLASS} flex-wrap disabled:cursor-default sm:flex-nowrap`}
              data-status={status}
              // A row offering its history, or the move to the official
              // connection, is named by what it shows — the provider, its
              // state and the action's words — so the spoken name carries the
              // visible label (WCAG 2.5.3); the history hint describes it.
              aria-describedby={offersHistory ? historyHintId : undefined}
              aria-label={
                status === 'authorize_owed'
                  ? t('providers.authorizeToKeepSyncing', name)
                  : offersHistory || status === 'migrate' || status === 'reconnect' || status === 'history_importing' || refused
                  ? undefined
                  : provider.connected
                  ? t('providers.isConnectedAria', name)
                  : isNonOAuth
                    ? `${provider.display_name} - ${provider.description}`
                    : t('providers.connectToAria', name)
              }
            >
              <span
                aria-hidden="true"
                data-testid={`provider-glyph-${provider.provider}`}
                className={`flex h-6 w-6 flex-shrink-0 items-center justify-center text-on-surface ${isNonOAuth ? 'opacity-60' : ''}`}
                style={glyphInk ? { color: glyphInk } : undefined}
              >
                <ProviderIcon providerId={provider.provider} className="h-5 w-5" />
              </span>
              <span className={`flex min-w-[60%] flex-1 flex-wrap items-baseline gap-x-2 sm:min-w-0 ${isNonOAuth ? 'opacity-60' : ''}`}>
                <span className="text-sm font-medium text-on-surface">{provider.display_name}</span>
                {!plain && <span className="min-w-0 truncate text-xs text-on-surface-variant">{provider.description}</span>}
              </span>
              <RowStatus status={status} provider={provider.provider} displayName={provider.display_name} refused={refused} />
              {isNonOAuth && !provider.connected && (
                <span className="flex-shrink-0 text-xs text-outline">{t('providers.demoBadge')}</span>
              )}
              {actionable && status !== 'authorize_owed' && !isNonOAuth && (
                <span className="flex-shrink-0 text-sm font-medium text-primary transition-colors group-hover:text-primary-hover">
                  {offersHistory
                    ? t(historyActionKey(provider))
                    : status === 'migrate'
                      ? t('app.connectProvider', name)
                      : status === 'reconnect'
                        ? t('app.reconnect')
                        : t('providers.connect')}
                </span>
              )}
              {offersHistory && (
                <span id={historyHintId} className="sr-only">
                  {t(HISTORY_ACTION_KEYS.explainer, name)}
                </span>
              )}
            </button>
            {!plain && (
              <div className="pb-2 pl-9 empty:hidden">
                <ProviderExportLink provider={provider} />
              </div>
            )}
          </div>
        );
      })}

      {/* Import your own data (Apple Health has no card): below the providers, never one of them */}
      {!plain && <ImportOwnDataLink />}

      {/* Skip and start chatting - last row */}
      {onSkip && (
        <button
          type="button"
          onClick={onSkip}
          disabled={isSkipPending}
          className={`${ROW_CLASS} border-t ghost-border-faint first:border-t-0`}
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

      {/* The provider's own account sign-in */}
      <SciotteLoginModal
        isOpen={signIn !== null}
        onClose={() => setSignIn(null)}
        onConnected={() => {
          refetch();
          // The sign-in completes in-process (no OAuth callback URL), so we have to
          // explicitly bust the onboarding-status cache here. Without this, the
          // App-level route guard stays on OnboardingConnectProvider until the
          // next poll tick (5s), stranding the user on the connected card.
          queryClient.invalidateQueries({ queryKey: QUERY_KEYS.user.onboardingStatus() });
          setSignIn(null);
        }}
        onPlanChanged={() => void refetch()}
        target={signIn?.target ?? 'strava'}
        consentRequired={signIn?.consentRequired ?? false}
        historyMode={signIn?.historyMode ?? false}
        onFallBackToOAuth={signIn?.fallback ? fallBackToOAuth(signIn.fallback) : undefined}
      />

      <ProviderNoticeDialog
        provider={noticeProvider}
        onCancel={() => setNoticeProvider(null)}
        onAccept={() => {
          const accepted = noticeProvider;
          setNoticeProvider(null);
          if (accepted) connectViaOAuth(accepted, served.find((p) => p.provider === accepted), true);
        }}
      />

      {ownAppSetup && (
        <OAuthAppSetupModal
          isOpen
          onClose={() => setOwnAppSetup(null)}
          onSaved={() => {
            setOwnAppSetup(null);
            launchOAuth(ownAppSetup.provider.provider, ownAppSetup.tosConsent);
          }}
          provider={ownAppSetup.provider.provider}
          displayName={ownAppSetup.provider.display_name}
          devPortalUrl={ownAppSetup.devPortalUrl}
          callbackUrl={ownAppSetup.provider.oauth_callback_url}
        />
      )}

      {/* Intervals.icu: the authorization when offered, and the API-key form */}
      <IntervalsIcuLinkModal
        isOpen={intervalsModal !== null}
        onClose={() => setIntervalsModal(null)}
        onAuthorize={
          intervalsModal?.offersAuthorize
            ? () => {
                setIntervalsModal(null);
                connectViaOAuth('intervals_icu', served.find((p) => p.provider === 'intervals_icu'));
              }
            : undefined
        }
        onConnected={() => {
          refetch();
          // A pasted key connects in-process (no OAuth callback), so bust the
          // onboarding-status cache explicitly — same reasoning as the sign-in above.
          queryClient.invalidateQueries({ queryKey: QUERY_KEYS.user.onboardingStatus() });
          setIntervalsModal(null);
        }}
      />
    </div>
  );
}

/** The dot and line a row shows for its card's state; nothing for a card still to connect. */
function RowStatus({
  status,
  provider,
  displayName,
  refused,
}: {
  status: CardStatus;
  provider: string;
  displayName: string;
  refused: boolean;
}) {
  const { t } = useTranslation();
  const name = { provider: displayName };
  const line = (dot: 'bg-warning' | 'bg-success' | null, text: string, testId?: string, live = false) => (
    <span
      className="inline-flex min-w-0 items-center gap-1.5 text-xs text-on-surface-variant sm:flex-shrink-0"
      data-testid={testId}
      role={live ? 'status' : undefined}
    >
      {dot && <span aria-hidden="true" className={`h-1.5 w-1.5 rounded-full ${dot}`} />}
      {text}
    </span>
  );
  if (refused) return line('bg-warning', t('providers.connectRefused', name), `provider-connect-refused-${provider}`, true);
  switch (status) {
    case 'authorize_owed':
      return line('bg-warning', t('providers.authorizeToKeepSyncing', name), `provider-authorization-owed-${provider}`);
    case 'reconnect':
      return line('bg-warning', t('providers.reconnectNeeded'), `provider-reconnect-needed-${provider}`);
    case 'migrate':
      return line('bg-warning', t('providers.migration.oauthRequired', name), `provider-migrate-${provider}`);
    case 'history_importing':
      return line(null, t(HISTORY_LINE_KEYS.importing, name), `provider-history-importing-${provider}`, true);
    case 'history_needed':
    case 'history_incomplete':
    case 'history_imported':
    case 'connected':
      return line('bg-success', t('providers.connected'));
    case 'connect':
      return null;
  }
}
