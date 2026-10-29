// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The page a provider connect's return shows when it has something to say: a refusal or a tombstone
// ABOUTME: Names the provider by its card, offers a tombstone's fresh start, and goes back to the route the connect started from

import { useQuery } from '@tanstack/react-query';
import type { ImportTombstoneRefusal } from '@pierre/ui-logic';
import { DravrLogo } from './DravrLogo';
import ProviderTombstoneNotice from './ProviderTombstoneNotice';
import { useTranslation } from '@pierre/i18n';
import { oauthApi, providersApi } from '../services/api';
import { QUERY_KEYS } from '../constants/queryKeys';

interface OAuthCallbackProps {
  provider: string;
  error?: string;
  /**
   * The tombstone a refused connect names: the provider account belonged to
   * a deleted Dravr account, and the page offers to start fresh with it.
   */
  tombstone?: ImportTombstoneRefusal;
  /** Leave the page for the route the connect started from. */
  onClose: () => void;
}

/**
 * The page a connect's return shows when it has something to say: a refusal
 * and its reason, or a tombstone to resolve. A grant goes straight back to the
 * page the connect started from (`settleOAuthReturn`) and never shows it; the
 * button here goes back there too, in this same tab.
 */
export default function OAuthCallback({ provider, error, tombstone, onClose }: OAuthCallbackProps) {
  const { t } = useTranslation();
  // The brand the provider's card shows, read from the same status query the
  // cards poll; the capitalised id until it answers.
  const { data: providersData } = useQuery({
    queryKey: QUERY_KEYS.providers.status(),
    queryFn: () => providersApi.getProvidersStatus(),
    enabled: tombstone !== undefined,
  });
  const providerDisplay =
    providersData?.providers?.find((p) => p.provider === provider)?.display_name ??
    provider.charAt(0).toUpperCase() + provider.slice(1);

  return (
    <div className="min-h-dvh bg-surface flex items-center justify-center px-4 pad-safe-top">
      <div className="max-w-md w-full bg-surface-container-lowest border ghost-border rounded-xl overflow-hidden">
        {/* The outcome as a hairline of colour — meaning, not decoration */}
        <div className="h-0.5 w-full bg-error" />

        <div className="px-8 py-10 text-center">
          {/* Logo */}
          <div className="mb-6">
            <DravrLogo size={64} className="mx-auto" />
          </div>

          <div className="text-lg font-bold text-on-surface mb-6">{t('shell.brandName')}</div>

          {/* Error icon */}
          <div className="w-16 h-16 bg-error-container rounded-full flex items-center justify-center mx-auto mb-4">
            <svg
              className="w-8 h-8 text-on-error-container"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M6 18L18 6M6 6l12 12"
              />
            </svg>
          </div>

          <h1 className="text-xl font-bold text-error mb-2">{t('shell.oauthConnectionFailed')}</h1>
          {tombstone ? (
            <div className="mb-4">
              <ProviderTombstoneNotice
                refusal={tombstone}
                providerName={providerDisplay}
                // The connect runs again in this same tab, so its return
                // lands here as the first one did.
                onCleared={() => {
                  window.location.href = oauthApi.authorizeUrl(provider);
                }}
              />
            </div>
          ) : (
            <p className="text-sm text-on-surface-variant mb-6">
              {error || t('app.failedConnectProviderAccount', { provider: providerDisplay })}
            </p>
          )}

          <button
            onClick={onClose}
            className={`${tombstone ? 'btn-secondary' : 'btn-primary'} w-full`}
          >
            {t('app.continue')}
          </button>
        </div>
      </div>
    </div>
  );
}
