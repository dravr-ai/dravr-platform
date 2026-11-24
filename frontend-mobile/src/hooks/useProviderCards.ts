// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The mobile binding of the provider-cards hook — the shared React Query read over this client's OAuth API
// ABOUTME: Connections, the onboarding connect step and the chat's connect flow hold one list, on the provider-status key

import { createProviderCardsHook } from '@pierre/ui-logic';
import { oauthApi } from '../services/api';

/**
 * The provider cards as the server lists them, and the two ways a screen
 * reads them again. The API is reached when a read runs, so importing it
 * touches no client.
 */
export const useProviderCards = createProviderCardsHook({
  getProvidersStatus: () => oauthApi.getProvidersStatus(),
});
