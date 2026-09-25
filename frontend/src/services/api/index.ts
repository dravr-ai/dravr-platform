// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: API service entry point - uses @pierre/api-client for shared modules
// ABOUTME: Web-only modules (admin, admin usage, dashboard, a2a, messaging, billing) remain local

import { pierreApi } from './client';

// Export the shared Pierre API instance for direct access
export { pierreApi } from './client';

// Export individual API modules - shared modules from @pierre/api-client
export const authApi = pierreApi.auth;
export const chatApi = pierreApi.chat;
export const coachesApi = pierreApi.coaches;
export const oauthApi = pierreApi.oauth;
export const storeApi = pierreApi.store;
export const userApi = pierreApi.user;
// End-user messaging channel linking (onboarding). Distinct from the web-only
// admin `messagingApi` (channel-config CRUD) re-exported from './messaging'.
export const messagingLinkApi = pierreApi.messaging;
export const notificationsApi = pierreApi.notifications;
export const groupsApi = pierreApi.groups;
export const featureFlagsApi = pierreApi.featureFlags;
// The « Style de coaching » cards, as the live persona-contract registry
// renders them.
export const personasApi = pierreApi.personas;
// The live string catalogue, overlaid on the embedded copy at start-up.
export const i18nApi = pierreApi.i18n;
// The calling user's quota counters, polled by the chat banner and the usage card.
export const usageApi = pierreApi.usage;

// The athlete's own training data for the Home page: recent activities, one
// activity's route, and the plan for today.
export const athleteApi = pierreApi.athlete;

// Providers API delegates to shared oauth module
export const providersApi = {
  getProvidersStatus: pierreApi.oauth.getProvidersStatus.bind(pierreApi.oauth),
  linkIntervalsIcu: pierreApi.oauth.linkIntervalsIcu.bind(pierreApi.oauth),
  disconnectIntervalsIcu: pierreApi.oauth.disconnectIntervalsIcu.bind(pierreApi.oauth),
};

// Export web-only modules from local implementations
export { dashboardApi } from './dashboard';
export { a2aApi } from './a2a';
export { adminApi } from './admin';
export { adminUsageApi } from './usage';
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
export type { FeatureFlagMap, KnownFeatureFlag, MeFeaturesResponse } from '@pierre/api-client';

// Export types from shared package
export type { Agent, StoreAgent } from '@pierre/shared-types';
export type { ExtendedProviderStatus as ProviderStatus, ProvidersStatusResponse } from '@pierre/shared-types';
