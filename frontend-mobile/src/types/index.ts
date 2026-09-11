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
} from '@pierre/shared-types';

// Coach types
export type {
  AgentCategory,
  AgentVisibility,
  PublishStatus,
  Agent,
  UpdateAgentRequest,
  ListAgentsResponse,
  StoreAgent,
  StoreAgentDetail,
  StoreMetadata,
  BrowseAgentsResponse,
  SearchAgentsResponse,
  InstallAgentResponse,
  UninstallAgentResponse,
  InstallationsResponse,
} from '@pierre/shared-types';

// Group coaching types
export type {
  GroupRole,
  CoachingGroup,
  GroupMember,
  GroupInvite,
  GroupAggregateStats,
  GroupWeeklyReport,
  GroupHealthFlag,
  HealthFlagSeverity,
  MemberFlag,
  CreateInviteRequest,
  UpdateGroupRequest,
  UpdateMemberRoleRequest,
  UpdatePeerConsentRequest,
  GroupMembersResponse,
  GroupInvitesResponse,
  GroupStatsResponse,
  GroupWeeklyReportResponse,
  GroupHealthFlagsResponse,
} from '@pierre/shared-types';
