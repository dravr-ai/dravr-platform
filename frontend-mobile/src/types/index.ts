// ABOUTME: TypeScript type definitions for Pierre Mobile app
// ABOUTME: Re-exports shared types for convenient importing

// Re-export all types from shared packages
// Auth types
export type {
  UserRole,
  UserStatus,
  UserTier,
  User,
  AdminUser,
  LoginResponse,
  RegisterResponse,
  FirebaseLoginResponse,
  ProviderStatus,
  ExtendedProviderStatus,
  ProvidersStatusResponse,
  OAuthApp,
  OAuthAppCredentials,
  OAuthProvider,
  McpToken,
} from '@pierre/shared-types';

// API types (chat, prompts)
export type {
  Conversation,
  Message,
  ActivityPillar,
  PromptCategory,
  PromptSuggestionsResponse,
} from '@pierre/shared-types';

// Coach types
export type {
  CoachCategory,
  CoachVisibility,
  PublishStatus,
  Coach,
  ForkCoachResponse,
  CreateCoachRequest,
  UpdateCoachRequest,
  ListCoachesResponse,
  StoreCoach,
  StoreCoachDetail,
  StoreMetadata,
  BrowseCoachesResponse,
  SearchCoachesResponse,
  CategoryCount,
  CategoriesResponse,
  InstallCoachResponse,
  UninstallCoachResponse,
  InstallationsResponse,
} from '@pierre/shared-types';

// Re-export social types (named exports to avoid pulling in runtime constants)
export type {
  FriendStatus,
  ShareVisibility,
  InsightType,
  ReactionType,
  TrainingPhase,
  FriendConnection,
  FriendWithInfo,
  DiscoverableUser,
  NotificationPreferences,
  UserSocialSettings,
  SharedInsight,
  InsightReaction,
  AdaptedInsight,
  FeedAuthor,
  ReactionCounts,
  FeedItem,
  SocialMetadata,
  ListFriendsResponse,
  PendingRequestsResponse,
  FriendConnectionResponse,
  SearchUsersResponse,
  FeedResponse,
  ShareInsightResponse,
  ListInsightsResponse,
  ListAdaptedInsightsResponse,
  ReactionResponse,
  AdaptInsightResponse,
  SocialSettingsResponse,
  ShareInsightRequest,
  UpdateSocialSettingsRequest,
  ListInsightsParams,
  InsightSuggestion,
  ListSuggestionsResponse,
  ShareFromActivityRequest,
  GetSuggestionsParams,
} from './social';

export {
  REACTION_EMOJIS,
  INSIGHT_TYPE_COLORS,
  INSIGHT_TYPE_LABELS,
} from './social';
