// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The mobile binding of the AI-consent hook — the shared React Query mutation over this client's OAuth API
// ABOUTME: One call either way (carnet#726) — the Privacy switch and the Connections card share it

import { createAiConsentHook } from '@pierre/ui-logic';
import { oauthApi } from '../services/api';

/**
 * Give or withdraw the account's consent to AI use of one provider's data,
 * then reread the provider cards. The API is reached when a change runs, so
 * importing it touches no client.
 */
export const useAiConsent = createAiConsentHook({
  grantAiConsent: (provider) => oauthApi.grantAiConsent(provider),
  withdrawAiConsent: (provider) => oauthApi.withdrawAiConsent(provider),
});
