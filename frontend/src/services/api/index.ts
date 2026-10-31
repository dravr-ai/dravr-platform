// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: API service entry point - uses @pierre/api-client for shared modules
// ABOUTME: Web-only modules (admin, admin usage, dashboard, a2a, messaging config) remain local

import { pierreApi } from './client';

// Export the shared Pierre API instance for direct access
export { pierreApi } from './client';

// Export individual API modules - shared modules from @pierre/api-client
export const authApi = pierreApi.auth;
export const chatApi = pierreApi.chat;
export const coachesApi = pierreApi.coaches;
// The human coach's roster for Coach Home: athletes with their ranked reasons, and "Traité".
export const coachRosterApi = pierreApi.coachRoster;
// The human coach's review of the agent's group replies: the page, a flag, a correction, the weekly count.
export const coachRepliesApi = pierreApi.coachReplies;
export const apiKeysApi = pierreApi.apiKeys;
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
// Subscription, invoices, plans, the quota snapshot, hosted checkout and portal.
export const billingApi = pierreApi.billing;

// The athlete's own training data for the Home page: recent activities, one
// activity's route, and the plan for today.
export const athleteApi = pierreApi.athlete;

// The athlete's shoes, their distance, and the pair each activity was run in.
export const gearApi = pierreApi.gear;

// Importing the athlete's own history from a service's export.
export const importsApi = pierreApi.imports;

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
export { coachAccessAdminApi } from './coachAccessAdmin';
// Accepting a mailed invitation: its link opens the web app only.
export { invitationsApi } from './invitations';
// Confirming an address from its mailed link, which opens the web app only.
export { emailConfirmationApi } from './emailConfirmation';
export { adminUsageApi } from './usage';
export { messagingApi } from './messaging';
// The personal thread's topics, which the web History lists (carnet#840).
export { topicsApi } from './topics';
// Home's Getting started, web-only until native Home has it (registre#1102).
export { gettingStartedApi } from './gettingStarted';
// The feedback thread with the Dravr team, web-only until the phone has it (registre#1241).
export { feedbackApi } from './feedback';
export type { ThreadTopic } from './topics';
export type { FeatureFlagMap, KnownFeatureFlag, MeFeaturesResponse } from '@pierre/api-client';

// Export types from shared package
export type { Agent, StoreAgent } from '@pierre/shared-types';
export type { ExtendedProviderStatus as ProviderStatus, ProvidersStatusResponse } from '@pierre/shared-types';
