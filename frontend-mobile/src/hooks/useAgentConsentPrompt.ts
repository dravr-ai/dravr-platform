// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The mobile binding of the one-time question letting the athlete's agent read its memory and their health records
// ABOUTME: The shared React Query read and answer over this client's OAuth API (carnet#1544)

import { createAgentConsentPromptHooks } from '@pierre/ui-logic';
import { oauthApi } from '../services/api';

/**
 * Read the question and record its answer. The API is reached when a hook
 * runs, so importing this touches no client.
 */
export const { useAgentConsentPrompt, useAnswerAgentConsentPrompt } = createAgentConsentPromptHooks({
  getAgentConsentPrompt: () => oauthApi.getAgentConsentPrompt(),
  answerAgentConsentPrompt: (answer) => oauthApi.answerAgentConsentPrompt(answer),
});
