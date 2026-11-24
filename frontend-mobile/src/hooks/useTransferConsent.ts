// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The mobile binding of the transfer-consent hook — the shared React Query mutation over this client's OAuth API
// ABOUTME: Gives or withdraws the consent to Dravr sending a provider the athlete's planned workouts (Garmin, carnet#1390)

import { createTransferConsentHook } from '@pierre/ui-logic';
import { oauthApi } from '../services/api';

/**
 * Give or withdraw the account's consent to Dravr sending one provider the
 * planned workouts, then reread the provider cards. The API is reached when
 * a change runs, so importing it touches no client.
 */
export const useTransferConsent = createTransferConsentHook({
  grantTransferConsent: (provider) => oauthApi.grantTransferConsent(provider),
  withdrawTransferConsent: (provider) => oauthApi.withdrawTransferConsent(provider),
});
