// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: API service entry point - uses @pierre/api-client for shared modules
// ABOUTME: Web-only modules (admin, keys, dashboard, a2a, usage) remain local

import { pierreApi } from './client';

// Export the shared Pierre API instance for direct access
export { pierreApi } from './client';

// Export individual API modules - shared modules from @pierre/api-client
export const authApi = pierreApi.auth;
export const chatApi = pierreApi.chat;
export const coachesApi = pierreApi.coaches;
export const oauthApi = pierreApi.oauth;
export const socialApi = pierreApi.social;
export const storeApi = pierreApi.store;
export const userApi = pierreApi.user;
// End-user messaging channel linking (onboarding). Distinct from the web-only
// admin `messagingApi` (channel-config CRUD) re-exported from './messaging'.
export const messagingLinkApi = pierreApi.messaging;
export const notificationsApi = pierreApi.notifications;
export const groupsApi = pierreApi.groups;

// Providers API delegates to shared oauth module
export const providersApi = {
  getProvidersStatus: pierreApi.oauth.getProvidersStatus.bind(pierreApi.oauth),
  linkIntervalsIcu: pierreApi.oauth.linkIntervalsIcu.bind(pierreApi.oauth),
  disconnectIntervalsIcu: pierreApi.oauth.disconnectIntervalsIcu.bind(pierreApi.oauth),
};

// Export web-only modules from local implementations
export { keysApi } from './keys';
export { dashboardApi } from './dashboard';
export { a2aApi } from './a2a';
export { adminApi } from './admin';
export { usageApi } from './usage';
export { messagingApi } from './messaging';
export { billingApi } from './billing';
export type {
  SubscriptionView,
  InvoicesResponse,
  QuotaCounter,
  MyQuotaResponse,
  PlanView,
  PlansResponse,
} from './billing';
export { featureFlagsApi } from './featureFlags';
export type { FeatureFlagMap, KnownFeatureFlag, MeFeaturesResponse } from './featureFlags';

// Export types from shared package
export type { Coach, StoreCoach } from '@pierre/shared-types';
export type { ExtendedProviderStatus as ProviderStatus, ProvidersStatusResponse } from '@pierre/shared-types';
