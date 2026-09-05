// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useEffect } from 'react';
import { DravrLogo } from './DravrLogo';
import { useTranslation } from '@pierre/i18n';

interface OAuthCallbackProps {
  provider: string;
  success: boolean;
  error?: string;
  onClose?: () => void;
}

/**
 * OAuth callback result page shown after OAuth redirect from provider.
 * Displays success or error state and allows user to continue to dashboard.
 */
export default function OAuthCallback({ provider, success, error, onClose }: OAuthCallbackProps) {
  const { t } = useTranslation();
  const providerDisplay = provider.charAt(0).toUpperCase() + provider.slice(1);

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
      timestamp: Date.now(),
    };
    localStorage.setItem('pierre_oauth_result', JSON.stringify(result));
  }, [provider, success, error]);


  return (
    <div className="min-h-dvh bg-surface flex items-center justify-center px-4">
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
              <div className="w-16 h-16 bg-activity rounded-full flex items-center justify-center mx-auto mb-4">
                <svg
                  className="w-8 h-8 text-activity"
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
                {providerDisplay} Connected
              </h1>
              <p className="text-sm text-on-surface-variant mb-6">
                {t('frag.your')} {providerDisplay} account has been successfully connected to Dravr.
              </p>
            </>
          ) : (
            <>
              {/* Error icon */}
              <div className="w-16 h-16 bg-error rounded-full flex items-center justify-center mx-auto mb-4">
                <svg
                  className="w-8 h-8 text-error"
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
              <p className="text-sm text-on-surface-variant mb-6">
                {error || t('app.failedConnectProviderAccount', { provider: providerDisplay })}
              </p>
            </>
          )}

          {onClose ? (
            <button
              onClick={onClose}
              className="btn-primary w-full"
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
