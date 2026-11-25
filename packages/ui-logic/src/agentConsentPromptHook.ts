// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query read and answer of the one-time question letting the athlete's agent read its memory and their health records
// ABOUTME: Answering rereads the question and both consent classes, so the privacy switches show what the answer gave (carnet#1544)

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { QUERY_KEYS, SENSITIVE_T2_CONSENT, SENSITIVE_T3_CONSENT } from '@pierre/shared-constants';
import type { OAuthApi } from '@pierre/api-client';
import type { AgentConsentPromptAnswer } from '@pierre/shared-types';

/**
 * Build the one-time question's hooks over one client's OAuth API.
 *
 * `useAgentConsentPrompt` reads whether it is due; `useAnswerAgentConsentPrompt`
 * records the answer and, on settle, rereads the question and the
 * `sensitive_t3` / `sensitive_t2` answers the privacy switches read.
 */
export function createAgentConsentPromptHooks(
  oauthApi: Pick<OAuthApi, 'getAgentConsentPrompt' | 'answerAgentConsentPrompt'>,
) {
  function useAgentConsentPrompt(enabled = true) {
    return useQuery({
      queryKey: QUERY_KEYS.providers.agentConsentPrompt(),
      queryFn: () => oauthApi.getAgentConsentPrompt(),
      enabled,
    });
  }

  function useAnswerAgentConsentPrompt() {
    const queryClient = useQueryClient();
    return useMutation({
      mutationFn: (answer: AgentConsentPromptAnswer) => oauthApi.answerAgentConsentPrompt(answer),
      onSettled: async () => {
        await Promise.all([
          queryClient.invalidateQueries({ queryKey: QUERY_KEYS.providers.agentConsentPrompt() }),
          queryClient.invalidateQueries({ queryKey: QUERY_KEYS.providers.aiConsent(SENSITIVE_T3_CONSENT) }),
          queryClient.invalidateQueries({ queryKey: QUERY_KEYS.providers.aiConsent(SENSITIVE_T2_CONSENT) }),
        ]);
      },
    });
  }

  return { useAgentConsentPrompt, useAnswerAgentConsentPrompt };
}
