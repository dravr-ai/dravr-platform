// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Hears a provider connection finishing in another window and says so in the open thread
// ABOUTME: Reads the OAuth callback's hand-off (a message, a storage write, or the result left for a returning tab) and reopens the thread the flow started from

import { useEffect, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import type { ProvidersStatusResponse } from '@pierre/shared-types';
import { QUERY_KEYS } from '../../constants/queryKeys';
import type { OAuthNotification } from './types';

/** What the open thread shows once a provider connection completes, and how it is dismissed. */
export interface OAuthCompletion {
  notification: OAuthNotification | null;
  dismiss: () => void;
}

/**
 * Listen for a completed provider OAuth flow.
 *
 * The callback page reports by `postMessage`, by a `pierre_oauth_result`
 * storage entry another tab sees, or by leaving that entry for this tab to
 * read when it regains focus. Whichever arrives first wins; the conversation
 * the flow started from (`pierre_oauth_conversation`) is reopened.
 */
export function useOAuthCompletion(onSelectConversation: (id: string | null) => void): OAuthCompletion {
  const queryClient = useQueryClient();
  const [notification, setNotification] = useState<OAuthNotification | null>(null);

  useEffect(() => {
    let isProcessingOAuth = false;

    const extractOAuthData = () => {
      const stored = localStorage.getItem('pierre_oauth_result');
      if (!stored) return null;
      localStorage.removeItem('pierre_oauth_result');

      try {
        const result = JSON.parse(stored);
        const fiveMinutesAgo = Date.now() - 5 * 60 * 1000;

        if (result.type === 'oauth_completed' && result.success && result.timestamp > fiveMinutesAgo) {
          const savedConversation = sessionStorage.getItem('pierre_oauth_conversation');
          if (savedConversation) sessionStorage.removeItem('pierre_oauth_conversation');
          return { result, savedConversation };
        } else if (result.timestamp <= fiveMinutesAgo) {
          sessionStorage.removeItem('pierre_oauth_conversation');
        }
      } catch {
        // Ignore parse errors
      }
      return null;
    };

    const processOAuthData = (data: { result: { provider: string }; savedConversation: string | null }) => {
      if (isProcessingOAuth) return;
      isProcessingOAuth = true;

      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.oauth.status() });
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.user.profile() });

      // The brand the provider's card shows, from the status the thread already
      // holds (as OAuthCallback reads it); the capitalised id until it answers.
      const { provider } = data.result;
      const providerDisplay =
        queryClient
          .getQueryData<ProvidersStatusResponse>(QUERY_KEYS.providers.status())
          ?.providers?.find((p) => p.provider === provider)?.display_name ??
        provider.charAt(0).toUpperCase() + provider.slice(1);
      setNotification({ provider: providerDisplay, timestamp: Date.now() });

      if (data.savedConversation) {
        onSelectConversation(data.savedConversation);
      }

      setTimeout(() => {
        isProcessingOAuth = false;
      }, 500);
    };

    const checkAndProcessOAuthResult = () => {
      const data = extractOAuthData();
      if (data) {
        processOAuthData(data);
      }
    };

    const handleOAuthMessage = (event: MessageEvent) => {
      // Validate origin to prevent cross-origin message injection
      if (event.origin !== window.location.origin) return;
      if (event.data?.type === 'oauth_completed') {
        const { provider, success } = event.data;
        if (success && !isProcessingOAuth) {
          const savedConversation = sessionStorage.getItem('pierre_oauth_conversation');
          if (savedConversation) sessionStorage.removeItem('pierre_oauth_conversation');
          processOAuthData({ result: { provider }, savedConversation });
        }
      }
    };

    const handleStorageChange = (event: StorageEvent) => {
      if (event.key === 'pierre_oauth_result' && event.newValue) {
        const data = extractOAuthData();
        if (data) {
          processOAuthData(data);
        }
      }
    };

    const handleVisibilityChange = () => {
      if (document.visibilityState === 'visible') {
        checkAndProcessOAuthResult();
      }
    };

    const handleFocus = () => {
      checkAndProcessOAuthResult();
    };

    window.addEventListener('message', handleOAuthMessage);
    window.addEventListener('storage', handleStorageChange);
    document.addEventListener('visibilitychange', handleVisibilityChange);
    window.addEventListener('focus', handleFocus);
    checkAndProcessOAuthResult();

    return () => {
      window.removeEventListener('message', handleOAuthMessage);
      window.removeEventListener('storage', handleStorageChange);
      document.removeEventListener('visibilitychange', handleVisibilityChange);
      window.removeEventListener('focus', handleFocus);
    };
  }, [queryClient, onSelectConversation]);

  return { notification, dismiss: () => setNotification(null) };
}
