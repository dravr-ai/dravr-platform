// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query mutation that gives or withdraws consent to AI use of one provider's data, per client's OAuth API
// ABOUTME: One call either way (carnet#726); rereads both provider reads and the key's own answer; a sibling query reads that answer

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { OAuthApi } from '@pierre/api-client';

/** One consent change: `allow` gives it, `!allow` withdraws it. */
export interface AiConsentChange {
  provider: string;
  allow: boolean;
}

/**
 * Build `useAiConsent` over one client's OAuth API.
 *
 * The mutation behind every AI-consent control — the privacy toggle and the
 * connection card's withdraw action alike. On settle it rereads both
 * provider-card queries, so every surface that shows a provider's consent
 * shows the server's answer rather than the tap, whichever control made it.
 */
export function createAiConsentHook(oauthApi: Pick<OAuthApi, 'grantAiConsent' | 'withdrawAiConsent'>) {
  return function useAiConsent() {
    const queryClient = useQueryClient();
    return useMutation({
      mutationFn: ({ provider, allow }: AiConsentChange) =>
        allow ? oauthApi.grantAiConsent(provider) : oauthApi.withdrawAiConsent(provider),
      onSettled: async (_answer, _error, { provider }) => {
        await Promise.all([
          queryClient.invalidateQueries({ queryKey: QUERY_KEYS.user.providerConnections() }),
          queryClient.invalidateQueries({ queryKey: QUERY_KEYS.providers.status() }),
          queryClient.invalidateQueries({ queryKey: QUERY_KEYS.providers.aiConsent(provider) }),
        ]);
      },
    });
  };
}

/**
 * Build `useAiConsentState` over one client's OAuth API: the account's answer
 * to one consent to AI use, read by key. A consent class such as
 * `sensitive_t2` has no provider card, so this is how its switch reads it;
 * {@link createAiConsentHook}'s mutation rereads it after every change.
 * `data` is `true` given, `false` not given, `null` when none is asked.
 */
export function createAiConsentStateHook(oauthApi: Pick<OAuthApi, 'getAiConsent'>) {
  return function useAiConsentState(provider: string, enabled = true) {
    return useQuery({
      queryKey: QUERY_KEYS.providers.aiConsent(provider),
      queryFn: () => oauthApi.getAiConsent(provider),
      enabled,
    });
  };
}
