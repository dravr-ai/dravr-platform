// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
// ABOUTME: API service entry point using @pierre/api-client with mobile adapter
// ABOUTME: Exports domain-based APIs with secure token storage via expo-secure-store

import AsyncStorage from '@react-native-async-storage/async-storage';
import * as SecureStore from 'expo-secure-store';
import {
  createHealthRecordsApi,
  createMobileAdapter,
  createPierreApi,
  ENDPOINTS,
  type LinkCompleteResponse,
  type PierreApiService,
  requireShape,
} from '@pierre/api-client';
import {
  parseDeviceHealthCounts,
  type DeviceHealthCounts,
  type DeviceHealthSource,
  type DeviceSamples,
} from '@pierre/shared-types';
import { getApiUrl } from '../apiUrl';

// Create the mobile platform adapter with secure token storage
const adapter = createMobileAdapter({
  asyncStorage: AsyncStorage,
  secureStorage: SecureStore,
  baseURL: getApiUrl(),
  timeout: 300000, // 5 minutes for slow LLM responses
});

// Create the full API service with all domain APIs
const api: PierreApiService = createPierreApi(adapter);

// The raw axios client, for the requests that are not a domain API: the
// health probe in useServerStatus, which polls /health with its own short
// timeout, and channelLinkApi below. The integration harness also swaps its
// adapter here to stub HTTP.
export const apiClient = api.axios;

// Subscribe to the adapter's auth-failure signal. AuthContext listens here to
// drop the signed-in user when a request comes back 401, so the app returns
// to the login screen instead of retrying with a dead token.
export const onAuthFailure = (listener: () => void): (() => void) => {
  return adapter.authFailure.subscribe(listener);
};

// The secure token storage the API client reads its bearer and refresh token
// from. Only the E2E-only seeded-session route writes it directly
// (app/(auth)/e2e/session.tsx, carnet#1201); every sign-in goes through authApi.
export const authStorage = adapter.authStorage;

// Export domain APIs for direct import
export const authApi = api.auth;
export const chatApi = api.chat;
export const coachesApi = api.coaches;
export const apiKeysApi = api.apiKeys;
export const oauthApi = api.oauth;
export const storeApi = api.store;
export const userApi = api.user;
// End-user messaging channel linking (onboarding): getAvailableChannels/initLink/listLinks/deleteLink.
export const messagingApi = api.messaging;
export const notificationsApi = api.notifications;
export const groupsApi = api.groups;
// Effective feature flags for the calling user — the gate both clients read.
export const featureFlagsApi = api.featureFlags;
// The Home tab's reads: recent activities, one activity's route, the plan for today.
export const athleteApi = api.athlete;
// The athlete's shoes, their distance, and the pair each activity was run in.
export const gearApi = api.gear;
// Importing the athlete's own history from a service's export.
export const importsApi = api.imports;
export const originalsApi = api.dataExport;
// The athlete's "Who read your data" feed: outside apps' reads and the coach's reads per day.
export const accessFeedApi = api.accessFeed;
// The athlete's health records: served records by kind, the lab review queue, uploaded lab reports.
// Built here over the shared client: createPierreApi leaves it out for the web's entry budget.
export const healthRecordsApi = createHealthRecordsApi(api.axios);
// The athlete's statutory access requests to a company: the letter they send and its legal deadline.
export const dataAccessRequestsApi = api.dataAccessRequests;
// The live string catalogue, overlaid on the embedded copy at start-up.
export const personasApi = api.personas;
export const i18nApi = api.i18n;
// The calling user's quota counters, polled by the chat banner and the usage card.
export const usageApi = api.usage;
// Subscription, invoices, plans, the quota snapshot, hosted checkout and portal.
export const billingApi = api.billing;
// The human coach's roster for Coach Home, and "Traité" on an attention reason.
export const coachRosterApi = api.coachRoster;
// The human coach's review of the agent's group replies: the page, a flag, a correction, the weekly count.
export const coachRepliesApi = api.coachReplies;
// Finishing a Slack/Discord link this app started (carnet#1580). Only the phone
// takes the provider's code back; the web finishes the link in the callback its
// signed-in browser lands on, so this lives here rather than in a shared domain.
export const channelLinkApi = {
  async completeLink(channel: string, state: string, code: string): Promise<LinkCompleteResponse> {
    const response = await apiClient.post<LinkCompleteResponse>(
      ENDPOINTS.MESSAGING.LINK_COMPLETE(channel),
      { state, code },
    );
    return response.data;
  },
};

/** A counts body, or an error naming the endpoint whose answer broke the contract. */
function deviceHealthCounts(body: unknown, endpoint: string): DeviceHealthCounts {
  return requireShape(parseDeviceHealthCounts(body), endpoint);
}

// The phone's own health store (HealthKit, Health Connect, owl phase 9). Only
// the phone has a store to read, so like channelLinkApi this lives here rather
// than in a shared domain the web would never call. The body is the samples as
// the OS handed them over; the server maps them.
export const deviceHealthApi = {
  /** One page of samples, at most `DEVICE_SAMPLES_MAX` of them, read from `samples.source`. */
  async syncDeviceSamples(samples: DeviceSamples): Promise<DeviceHealthCounts> {
    const endpoint = ENDPOINTS.DEVICE_HEALTH.SYNC(samples.source);
    const response = await apiClient.post<unknown>(endpoint, samples);
    return deviceHealthCounts(response.data, endpoint);
  },

  /** Everything synced from `source`: activities, nights, days and self-reports. */
  async deleteDeviceData(source: DeviceHealthSource): Promise<DeviceHealthCounts> {
    const endpoint = ENDPOINTS.DEVICE_HEALTH.DATA(source);
    const response = await apiClient.delete<unknown>(endpoint);
    return deviceHealthCounts(response.data, endpoint);
  },
};
