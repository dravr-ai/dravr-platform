// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useEffect } from 'react';
import { DravrLogo } from './DravrLogo';

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
    <div className="min-h-screen bg-surface-container-low flex items-center justify-center px-4">
      <div className="max-w-md w-full bg-surface rounded-xl shadow-lg overflow-hidden">
        {/* Top gradient bar */}
        <div
          className={`h-1 w-full ${
            success
              ? 'bg-gradient-to-r from-pierre-activity to-pierre-activity-dark'
              : 'bg-gradient-to-r from-red-500 to-red-600'
          }`}
        />

        <div className="px-8 py-10 text-center">
          {/* Logo */}
          <div className="mb-6">
            <DravrLogo size={80} className="mx-auto" />
          </div>

          <div className="text-lg font-bold text-on-surface mb-6">Dravr</div>

          {success ? (
            <>
              {/* Success icon */}
              <div className="w-16 h-16 bg-pierre-activity-light rounded-full flex items-center justify-center mx-auto mb-4">
                <svg
                  className="w-8 h-8 text-pierre-activity"
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

              <h1 className="text-xl font-bold text-pierre-activity mb-2">
                {providerDisplay} Connected
              </h1>
              <p className="text-sm text-on-surface-variant mb-6">
                Your {providerDisplay} account has been successfully connected to Pierre.
              </p>
            </>
          ) : (
            <>
              {/* Error icon */}
              <div className="w-16 h-16 bg-pierre-red-100 rounded-full flex items-center justify-center mx-auto mb-4">
                <svg
                  className="w-8 h-8 text-pierre-red-500"
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

              <h1 className="text-xl font-bold text-pierre-red-600 mb-2">Connection Failed</h1>
              <p className="text-sm text-on-surface-variant mb-6">
                {error || `Failed to connect your ${providerDisplay} account. Please try again.`}
              </p>
            </>
          )}

          {onClose ? (
            <button
              onClick={onClose}
              className="btn-primary w-full"
            >
              Continue to Dashboard
            </button>
          ) : (
            <p className="text-xs text-on-surface-variant">
              You can close this tab and return to your conversation.
            </p>
          )}
        </div>
      </div>
    </div>
  );
}
