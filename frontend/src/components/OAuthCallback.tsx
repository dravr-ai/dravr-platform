// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useEffect } from 'react';
import { useQuery } from '@tanstack/react-query';
import type { ImportTombstoneRefusal } from '@pierre/ui-logic';
import { DravrLogo } from './DravrLogo';
import ProviderTombstoneNotice from './ProviderTombstoneNotice';
import { useTranslation } from '@pierre/i18n';
import { oauthApi, providersApi } from '../services/api';
import { QUERY_KEYS } from '../constants/queryKeys';

interface OAuthCallbackProps {
  provider: string;
  success: boolean;
  error?: string;
  /**
   * The tombstone a refused connect names: the provider account belonged to
   * a deleted Dravr account, and the page offers to start fresh with it.
   */
  tombstone?: ImportTombstoneRefusal;
  onClose?: () => void;
}

/**
 * OAuth callback result page shown after OAuth redirect from provider.
 * Displays success or error state and allows user to continue to dashboard.
 */
export default function OAuthCallback({ provider, success, error, tombstone, onClose }: OAuthCallbackProps) {
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

  // Store OAuth result in localStorage so ChatTab can display connection status
  // Security: Only store non-sensitive data - error messages may contain sensitive info
  useEffect(() => {
    const result = {
      type: 'oauth_completed',
      provider,
      success,
      // Don't store raw error messages in localStorage - they may contain sensitive info
      // Store a generic error indicator instead
      hasError: !success && !!error,
      // A refusal the athlete resolves here, in this tab: the opener must not
      // fall back to another way of connecting the same account.
      tombstoned: tombstone !== undefined,
      timestamp: Date.now(),
    };
    localStorage.setItem('pierre_oauth_result', JSON.stringify(result));
  }, [provider, success, error, tombstone]);


  return (
    <div className="min-h-dvh bg-surface flex items-center justify-center px-4 pad-safe-top">
      <div className="max-w-md w-full bg-surface-container-lowest border ghost-border rounded-xl overflow-hidden">
        {/* The outcome as a hairline of colour — meaning, not decoration */}
        <div className={`h-0.5 w-full ${success ? 'bg-success' : 'bg-error'}`} />

        <div className="px-8 py-10 text-center">
          {/* Logo */}
          <div className="mb-6">
            <DravrLogo size={64} className="mx-auto" />
          </div>

          <div className="text-lg font-bold text-on-surface mb-6">{t('shell.brandName')}</div>

          {success ? (
            <>
              {/* Success icon */}
              <div className="w-16 h-16 bg-activity/15 rounded-full flex items-center justify-center mx-auto mb-4">
                <svg
                  className="w-8 h-8 text-on-activity-container"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                >
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M5 13l4 4L19 7"
                  />
                </svg>
              </div>

              <h1 className="text-xl font-bold text-activity mb-2">
                {t('app.providerConnected', { provider: providerDisplay })}
              </h1>
              <p className="text-sm text-on-surface-variant mb-6">
                {t('app.providerAccountConnected', { provider: providerDisplay })}
              </p>
            </>
          ) : (
            <>
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
            </>
          )}

          {onClose ? (
            <button
              onClick={onClose}
              className={`${tombstone ? 'btn-secondary' : 'btn-primary'} w-full`}
            >
              {t('shell.oauthContinueToDashboard')}
            </button>
          ) : (
            <p className="text-xs text-on-surface-variant">
              {t('shell.oauthCloseTabHint')}
            </p>
          )}
        </div>
      </div>
    </div>
  );
}
