// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: One fitness provider's row in Settings' connections: its glyph, name, state line, history, export link and actions
// ABOUTME: What the row shows and offers comes from the shared cardStatus; the caller decides how a tap connects

import { useTranslation } from '@pierre/i18n';
import {
  AI_CONSENT_KEYS,
  HISTORY_ACTION_KEYS,
  IMPORT_HISTORY_ON_CARD_KEYS,
  cardStatus,
  coachPlatformName,
  countKey,
  delegationRefusalKey,
  historyActionKey,
  providerGlyphInk,
  providerScopeLabelKey,
} from '@pierre/shared-constants';
import type { ImportsOverview } from '@pierre/shared-types';
import { cardExportSource, importedHistoryCount, shownHistoryChoices } from '@pierre/ui-logic';
import type { ProviderStatus } from '../../services/api';
import { useTheme } from '../../hooks/useTheme';
import { Badge, Button } from '../ui';
import ProviderHistoryLine from '../ProviderHistoryLine';
import ProviderExportLink from '../import/ProviderExportLink';
import { importWindowHash, openImportWindow } from '../import/openImportWindow';

/** The DOM id of a provider's history line, which its history button names as its description. */
const historyLineId = (provider: string): string => `provider-history-line-${provider}`;

interface ProviderRowProps {
  provider: ProviderStatus;
  /** The athlete's imports, for the count of history the card's export brought in. */
  importsOverview: ImportsOverview | undefined;
  /** The card's connect is on its way to the provider. */
  isConnecting: boolean;
  aiConsentPending: boolean;
  /** Connect, reconnect, authorize or move the card, the way its plan says. */
  onConnect: () => void;
  /** Open the card's "Bring in your history" sheet. */
  onAddHistory: () => void;
  onDisconnect: () => void;
  onWithdrawAiConsent: () => void;
}

export default function ProviderRow({
  provider,
  importsOverview,
  isConnecting,
  aiConsentPending,
  onConnect,
  onAddHistory,
  onDisconnect,
  onWithdrawAiConsent,
}: ProviderRowProps) {
  const { t, language } = useTranslation();
  const { scheme } = useTheme();
  const name = { provider: provider.display_name };
  const status = cardStatus(provider);
  const glyphInk = providerGlyphInk(provider.provider, scheme);
  // A connection the user's group coach serves: confirmed, it is read through
  // the coach's account; proposed, it waits for the user's answer in the group.
  const delegation = provider.delegation;
  const isDelegated = provider.connected && delegation?.status === 'confirmed';
  // The history the card's export brought in, on every card the server names
  // an export source for; it opens that import.
  const archiveSource = cardExportSource(provider);
  const importedCount = archiveSource ? importedHistoryCount(importsOverview, archiveSource) : 0;
  // A card offers its history only while it has a way to bring it in that
  // this build can show; with none, it offers neither the action nor the sheet.
  const offersHistory =
    (status === 'history_needed' || status === 'history_incomplete') && shownHistoryChoices(provider).length > 0;
  // A connected card whose connection still serves: its actions are the
  // history, the AI consent and the disconnect, and a card asked to move to its
  // official connection also offers that move. Every other card — to connect,
  // to reconnect, owing its authorization — shows its connect action.
  const servesNow = provider.connected && status !== 'reconnect' && status !== 'authorize_owed';
  const connectable =
    provider.requires_oauth || provider.provider.startsWith('sciotte') || provider.provider === 'intervals_icu';

  const connectLabel =
    status === 'authorize_owed'
      ? t('providers.authorizeAction')
      : status === 'reconnect'
        ? t('settingsUi.reconnect')
        : status === 'migrate'
          ? t('app.connectProvider', name)
          : t('shell.intervalsConnectAction');

  return (
    <div
      data-testid={`provider-row-${provider.provider}`}
      data-status={status}
      className="border-t ghost-border-faint py-3 first:border-t-0 first:pt-0 last:pb-0"
    >
      <div className="flex items-center gap-3">
        {/* The provider's colour lives in its letter, not in a tile — in a
            scheme where the brand clears the 3:1 icon floor on the canvas;
            elsewhere the letter takes the body ink. */}
        <span
          aria-hidden="true"
          data-testid={`provider-glyph-${provider.provider}`}
          className="flex h-6 w-6 flex-shrink-0 items-center justify-center font-display text-sm font-bold text-on-surface"
          style={glyphInk ? { color: glyphInk } : undefined}
        >
          {provider.display_name.charAt(0)}
        </span>
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <p className="font-medium text-on-surface">{provider.display_name}</p>
            {provider.account_role === 'coach' && (
              <span data-testid={`provider-coach-account-${provider.provider}`}>
                <Badge variant="secondary">{t('humanCoach.trainingpeaksAccount')}</Badge>
              </span>
            )}
            {isDelegated ? (
              <span
                className="inline-flex items-center gap-1.5 text-xs text-on-surface-variant"
                data-testid={`provider-delegated-${provider.provider}`}
              >
                <span
                  aria-hidden="true"
                  className={`h-2 w-2 rounded-full ${delegation.coach_needs_reauth || delegation.read_refused ? 'bg-warning' : 'bg-success'}`}
                />
                {delegation.read_refused
                  ? t(delegationRefusalKey(delegation.read_refused), {
                      platform: coachPlatformName(provider.provider),
                    })
                  : delegation.coach_needs_reauth
                    ? t('delegation.coachReconnectNeeded', {
                        platform: coachPlatformName(provider.provider),
                        coach: delegation.coach_display_name,
                      })
                    : t('providers.connectedThrough', { coach: delegation.coach_display_name })}
              </span>
            ) : (
              <StatusPill status={status} provider={provider.provider} displayName={provider.display_name} />
            )}
          </div>
          <p className="text-xs text-on-surface-variant truncate">{provider.description}</p>
          <ProviderHistoryLine provider={provider} id={historyLineId(provider.provider)} />
          {archiveSource && importedCount > 0 ? (
            <button
              type="button"
              onClick={() => openImportWindow(importWindowHash(archiveSource))}
              className="text-xs font-medium text-primary hover:text-primary-hover mt-0.5"
              data-testid={`provider-imported-history-${provider.provider}`}
            >
              {t(countKey(importedCount, language, IMPORT_HISTORY_ON_CARD_KEYS), { count: importedCount })}{' '}
              <span aria-hidden="true">›</span>
            </button>
          ) : (
            <div>
              <ProviderExportLink provider={provider} />
            </div>
          )}
          {delegation?.status === 'proposed' && (
            <p className="text-xs text-on-surface-variant mt-0.5" data-testid={`provider-pending-link-${provider.provider}`}>
              {t('providers.pendingLink', {
                coach: delegation.coach_display_name,
                group: delegation.group_name,
              })}
            </p>
          )}
          {provider.account_role === 'coach' && (
            <p className="text-xs text-outline mt-0.5">{t('humanCoach.trainingpeaksAccountHint')}</p>
          )}
          {provider.capabilities.length > 0 && (
            <p className="text-xs text-outline mt-0.5">
              {provider.capabilities
                .map((scope) => {
                  // A slug the catalogue has no word for prints as itself — a
                  // provider that starts advertising a new capability must not
                  // show a missing-key string.
                  const key = providerScopeLabelKey(scope);
                  return key ? t(key) : scope;
                })
                .join(', ')}
            </p>
          )}
        </div>
        <div className="flex-shrink-0">
          {servesNow ? (
            <div className="flex flex-wrap items-center justify-end gap-1">
              {/* Withdrawing the consent to AI use is one tap; the connection
                  stays, and the card then asks for the authorization again to
                  give it back (carnet#726). */}
              {provider.ai_consent === true && (
                <Button
                  variant="tertiary"
                  size="sm"
                  onClick={onWithdrawAiConsent}
                  disabled={aiConsentPending}
                  data-testid={`provider-ai-consent-withdraw-${provider.provider}`}
                >
                  {t(AI_CONSENT_KEYS.withdraw)}
                </Button>
              )}
              {/* A push-fed grant (Garmin) carries only what comes after it:
                  while the history is needed, or its last copy did not finish,
                  the card opens the sheet that chooses how to bring it in. */}
              {offersHistory && (
                <Button
                  variant="outline"
                  size="sm"
                  onClick={onAddHistory}
                  data-testid={`provider-history-add-${provider.provider}`}
                  aria-describedby={historyLineId(provider.provider)}
                  title={t(HISTORY_ACTION_KEYS.addAria, name)}
                >
                  {t(historyActionKey(provider))}
                </Button>
              )}
              {status === 'migrate' && (
                <Button
                  variant="outline"
                  size="sm"
                  onClick={onConnect}
                  loading={isConnecting}
                  data-testid={`provider-connect-${provider.provider}`}
                >
                  {connectLabel}
                </Button>
              )}
              {connectable && (
                <Button
                  variant="tertiary"
                  size="sm"
                  onClick={onDisconnect}
                  className="text-error"
                  data-testid={`provider-disconnect-${provider.provider}`}
                >
                  {/* Disconnecting a delegated connection ends the coach's link. */}
                  {isDelegated ? t('delegation.unlink') : t('settingsUi.disconnect')}
                </Button>
              )}
            </div>
          ) : connectable ? (
            <Button
              variant="outline"
              size="sm"
              onClick={onConnect}
              loading={isConnecting}
              data-testid={`provider-connect-${provider.provider}`}
            >
              {connectLabel}
            </Button>
          ) : (
            <Badge variant="secondary">{t('settingsUi.manual')}</Badge>
          )}
        </div>
      </div>
    </div>
  );
}

/** The dot and words beside the name for the card's state; nothing for a card still to connect. */
function StatusPill({ status, provider, displayName }: { status: ReturnType<typeof cardStatus>; provider: string; displayName: string }) {
  const { t } = useTranslation();
  const pill = (dot: 'bg-warning' | 'bg-success', text: string, testId?: string) => (
    <span className="inline-flex items-center gap-1.5 text-xs text-on-surface-variant" data-testid={testId}>
      <span aria-hidden="true" className={`h-2 w-2 rounded-full ${dot}`} />
      {text}
    </span>
  );
  switch (status) {
    case 'authorize_owed':
      return pill('bg-warning', t('providers.authorizeToKeepSyncing', { provider: displayName }), `provider-authorization-owed-${provider}`);
    case 'reconnect':
      return pill('bg-warning', t('providers.reconnectNeeded'), `provider-reconnect-needed-${provider}`);
    case 'migrate':
      return pill('bg-warning', t('providers.migration.oauthRequired', { provider: displayName }), `provider-migrate-${provider}`);
    case 'connect':
      return null;
    default:
      return pill('bg-success', t('settingsUi.connected'));
  }
}
