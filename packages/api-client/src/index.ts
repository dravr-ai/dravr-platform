// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Main entry point for @pierre/api-client shared package
// ABOUTME: Exports API factories, domain APIs, and platform adapters

// Re-export types
export type {
  PlatformAdapter,
  AuthStorage,
  AuthFailureHandler,
  ResponseBodyReader,
  HttpClientConfig,
  ApiClientOptions,
  ApiMetadata,
  CursorPaginatedResponse,
  OffsetPaginatedResponse,
} from './types/platform';

// Re-export core
export { createApiClient, createAxiosClient } from './core/client';
export type { ApiClient } from './core/client';
export { ENDPOINTS } from './core/endpoints';
export type { EndpointKeys } from './core/endpoints';

// Re-export domain API factories
export { createAuthApi } from './domains/auth';
export type { AuthApi, LoginCredentials, RegisterCredentials } from './domains/auth';

export { createChatApi, replySceneBlocks } from './domains/chat';
export type {
  ChatApi,
  Conversation,
  Message,
  ConversationsResponse,
  MessagesResponse,
  TurnEnvelope,
  SendTurnOptions,
  ChatMessageAction,
  CreateConversationOptions,
} from './domains/chat';
// The turn envelope's own shapes live in @pierre/shared-types, where web and
// mobile read them from one declaration; re-exported here so a client that
// already imports the chat API does not need a second import path.
export type {
  AssistantTurn,
  ReplyBlock,
  ReplyNotice,
  ReplyVerdictChip,
  TurnTelemetry,
  TurnProgress,
} from '@pierre/shared-types';
export { parseTurnBody, readEventStream, TurnRequestError } from './core/turn-stream';
export type { TurnCallbacks, TurnProgressSink, SseFrame } from './core/turn-stream';
export type { ClaimVerdict, ChatVerdictsResponse } from './domains/chat';

export { createCoachesApi } from './domains/coaches';
export type {
  CoachesApi,
  Coach,
  ListCoachesOptions,
} from './domains/coaches';

export { createOAuthApi } from './domains/oauth';
export type {
  OAuthApi,
  OAuthProvider,
  OAuthStatusResponse,
  MobileOAuthInitResponse,
} from './domains/oauth';

export { createStoreApi } from './domains/store';
export type { StoreApi, BrowseOptions } from './domains/store';

export { createUserApi } from './domains/user';
export type {
  UserApi,
  UserStats,
  SupportedLocale,
  UpdateLocaleResponse,
  McpToken,
  McpTokensResponse,
  CreateMcpTokenRequest,
  UserOAuthApp,
  LlmProviderStatus,
  LlmCredentialSummary,
  LlmSettingsResponse,
  SaveLlmCredentialsRequest,
  SaveLlmCredentialsResponse,
  MemoryFactRow,
  MemoryFactListResponse,
  ForgetMemoryFactResponse,
} from './domains/user';

export { createMessagingApi } from './domains/messaging';
export type {
  MessagingApi,
  AvailableChannel,
  LinkInitResponse,
  ChannelLink,
} from './domains/messaging';

export { createNotificationsApi } from './domains/notifications';
export type {
  NotificationsApi,
  DeviceToken,
  RegisterDeviceTokenRequest,
  NotificationPreferencesResponse,
  UpdateNotificationPreferenceRequest,
  NotificationPreferenceItem,
  NotificationFeedResponse,
  UnreadCountResponse,
  MarkAllReadResponse,
  ListNotificationsParams,
} from './domains/notifications';

export {
  createFeatureFlagsApi,
  FEATURE_KEYS,
  FALLBACK_FEATURE_FLAGS,
  mergeFeatureFlags,
} from './domains/featureFlags';
export type {
  FeatureFlagsApi,
  FeatureFlagMap,
  KnownFeatureFlag,
  MeFeaturesResponse,
} from './domains/featureFlags';

export { createGroupsApi } from './domains/groups';
export type {
  GroupsApi,
  CoachingGroup,
  GroupMember,
  GroupInvite,
  GroupAggregateStats,
  GroupWeeklyReport,
  GroupHealthFlag,
  GroupMembersResponse,
  GroupInvitesResponse,
  GroupStatsResponse,
  GroupWeeklyReportResponse,
  GroupHealthFlagsResponse,
} from './domains/groups';

// Re-export platform adapters
export { createMobileAdapter } from './adapters/mobile';
export type { AsyncStorageLike, MobileAdapterOptions } from './adapters/mobile';
export { createWebAdapter } from './adapters/web';
export type { WebAdapterOptions } from './adapters/web';

// Import for unified API service
import type { AxiosInstance } from 'axios';
import type { PlatformAdapter } from './types/platform';
import { createAxiosClient } from './core/client';
import { createAuthApi } from './domains/auth';
import { createChatApi } from './domains/chat';
import { createCoachesApi } from './domains/coaches';
import { createOAuthApi } from './domains/oauth';
import { createStoreApi } from './domains/store';
import { createUserApi } from './domains/user';
import { createMessagingApi } from './domains/messaging';
import { createNotificationsApi } from './domains/notifications';
import { createGroupsApi } from './domains/groups';
import { createFeatureFlagsApi } from './domains/featureFlags';
export { createPersonasApi } from './domains/personas';
export type { PersonasApi, PersonaCard, PersonaRule, PersonasResponse } from './domains/personas';
import { createPersonasApi } from './domains/personas';
export { createI18nApi } from './domains/i18n';
export type { I18nApi, I18nBundle, I18nBundleResult } from './domains/i18n';
import { createI18nApi } from './domains/i18n';

/**
 * Complete API service combining all domain APIs.
 * Provides a unified interface for all Pierre API operations.
 */
export interface PierreApiService {
  /** Authentication API */
  auth: ReturnType<typeof createAuthApi>;
  /** Chat API */
  chat: ReturnType<typeof createChatApi>;
  /** Coaches API */
  coaches: ReturnType<typeof createCoachesApi>;
  /** OAuth API */
  oauth: ReturnType<typeof createOAuthApi>;
  /** Store API */
  store: ReturnType<typeof createStoreApi>;
  /** User API */
  user: ReturnType<typeof createUserApi>;
  /** Messaging API (end-user channel linking for onboarding) */
  messaging: ReturnType<typeof createMessagingApi>;
  /** Notifications API */
  notifications: ReturnType<typeof createNotificationsApi>;
  /** Groups API */
  groups: ReturnType<typeof createGroupsApi>;
  /** Feature flags API */
  featureFlags: ReturnType<typeof createFeatureFlagsApi>;
  /** The « Style de coaching » cards, rendered from the live persona-contract registry */
  personas: ReturnType<typeof createPersonasApi>;
  /** The live string catalogue, overlaid on the embedded copy at start-up and on language change */
  i18n: ReturnType<typeof createI18nApi>;
  /** Underlying axios instance for custom requests */
  axios: AxiosInstance;
  /** Platform adapter */
  adapter: PlatformAdapter;
}

/**
 * Creates a complete API service with all domain APIs.
 *
 * @example
 * // Web usage
 * import { createPierreApi } from '@pierre/api-client';
 * import { createWebAdapter } from '@pierre/api-client/adapters/web';
 *
 * const adapter = createWebAdapter();
 * const api = createPierreApi(adapter);
 *
 * // Use domain APIs
 * const coaches = await api.coaches.list();
 * const user = await api.auth.login({ email, password });
 *
 * @example
 * // Mobile usage
 * import { createPierreApi } from '@pierre/api-client';
 * import { createMobileAdapter } from '@pierre/api-client/adapters/mobile';
 * import AsyncStorage from '@react-native-async-storage/async-storage';
 *
 * const adapter = createMobileAdapter({ asyncStorage: AsyncStorage });
 * const api = createPierreApi(adapter);
 */
export function createPierreApi(adapter: PlatformAdapter): PierreApiService {
  const axios = createAxiosClient(adapter);

  return {
    auth: createAuthApi(axios, adapter.authStorage),
    chat: createChatApi(axios, adapter),
    coaches: createCoachesApi(axios),
    oauth: createOAuthApi(axios),
    store: createStoreApi(axios),
    user: createUserApi(axios),
    messaging: createMessagingApi(axios),
    notifications: createNotificationsApi(axios),
    groups: createGroupsApi(axios),
    featureFlags: createFeatureFlagsApi(axios),
    personas: createPersonasApi(axios),
    i18n: createI18nApi(axios),
    axios,
    adapter,
  };
}
