// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
// ABOUTME: API service entry point using @pierre/api-client with mobile adapter
// ABOUTME: Exports domain-based APIs with secure token storage via expo-secure-store

import AsyncStorage from '@react-native-async-storage/async-storage';
import * as SecureStore from 'expo-secure-store';
import {
  createMobileAdapter,
  createPierreApi,
  type PierreApiService,
} from '@pierre/api-client';
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

// The raw axios client, for the one request that is not a domain API: the
// health probe in useServerStatus, which polls /health with its own short
// timeout. The integration harness also swaps its adapter here to stub HTTP.
export const apiClient = api.axios;

// Subscribe to the adapter's auth-failure signal. AuthContext listens here to
// drop the signed-in user when a request comes back 401, so the app returns
// to the login screen instead of retrying with a dead token.
export const onAuthFailure = (listener: () => void): (() => void) => {
  return adapter.authFailure.subscribe(listener);
};

// Export domain APIs for direct import
export const authApi = api.auth;
export const chatApi = api.chat;
export const coachesApi = api.coaches;
export const oauthApi = api.oauth;
export const storeApi = api.store;
export const userApi = api.user;
// End-user messaging channel linking (onboarding): getAvailableChannels/initLink/listLinks/deleteLink.
export const messagingApi = api.messaging;
export const notificationsApi = api.notifications;
export const groupsApi = api.groups;
// Effective feature flags for the calling user — the gate both clients read.
export const featureFlagsApi = api.featureFlags;
