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
  CoachCategory,
  CoachVisibility,
  PublishStatus,
  Coach,
  FieldChange,
  ForkCoachResponse,
  ImportCoachResponse,
  ImportPreviewResponse,
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

// Group coaching types
export type {
  GroupRole,
  CoachingGroup,
  GroupMember,
  GroupInvite,
  GroupSummary,
  GroupAggregateStats,
  GroupWeeklyReport,
  GroupHealthFlag,
  HealthFlagSeverity,
  MemberFlag,
  CreateGroupRequest,
  UpdateGroupRequest,
  UpdateMemberRoleRequest,
  UpdatePeerConsentRequest,
  JoinGroupRequest,
  ListGroupsResponse,
  GroupMembersResponse,
  GroupInvitesResponse,
  GroupStatsResponse,
  GroupWeeklyReportResponse,
  GroupHealthFlagsResponse,
} from '@pierre/shared-types';
