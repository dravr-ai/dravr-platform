// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Gives or withdraws the account's consent to AI use of one provider's data, then rereads the provider cards
// ABOUTME: One call either way (carnet#726) — the privacy settings toggle and the connection card's action share it

import { useMutation, useQueryClient } from '@tanstack/react-query';
import { oauthApi } from '../services/api';
import { QUERY_KEYS } from '../constants/queryKeys';

/** One consent change: `allow` gives it, `!allow` withdraws it. */
export interface AiConsentChange {
  provider: string;
  allow: boolean;
}

/**
 * The mutation behind every AI-consent control. On settle it rereads both
 * provider-card queries, so the toggle, the card and the connect surfaces
 * show the server's answer rather than the click.
 */
export function useAiConsent() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ provider, allow }: AiConsentChange) =>
      allow ? oauthApi.grantAiConsent(provider) : oauthApi.withdrawAiConsent(provider),
    onSettled: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: QUERY_KEYS.user.providerConnections() }),
        queryClient.invalidateQueries({ queryKey: QUERY_KEYS.providers.status() }),
      ]);
    },
  });
}
