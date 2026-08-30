// ABOUTME: Main entry point for @pierre/shared-types package
// ABOUTME: Re-exports all shared types for convenient importing

// Coach types (AI coaching personas, store, versions)
export type {
  ActivityDataRequirements,
  DataRequirements,
  CoachCategory,
  CoachVisibility,
  PublishStatus,
  Coach,
  UpdateCoachRequest,
  CoachMetadata,
  ListCoachesResponse,
  StoreCoach,
  StoreCoachDetail,
  StoreMetadata,
  BrowseCoachesResponse,
  SearchCoachesResponse,
  InstallCoachResponse,
  UninstallCoachResponse,
  InstallationsResponse,
  CoachAssignment,
  AssignCoachResponse,
  UnassignCoachResponse,
  ListAssignmentsResponse,
  SportShare,
  SportProfileSummary,
  ProposedCoach,
  CoachProposalResponse,
} from './coaches.js';

// Auth types (users, login, OAuth)
export type {
  UserRole,
  UserStatus,
  UserTier,
  CoachingPersona,
  User,
  AdminUser,
  LoginResponse,
  RegisterResponse,
  FirebaseLoginResponse,
  SessionResponse,
  ProviderStatus,
  ExtendedProviderStatus,
  ProvidersStatusResponse,
  OAuthApp,
  OAuthAppCredentials,
  OAuthProvider,
  OAuthGrant,
  McpToken,
  UserManagementResponse,
  ApproveUserRequest,
  SuspendUserRequest,
  ForgotPasswordResponse,
  ResetPasswordResponse,
  ThemePreference,
  UpdateThemeRequest,
} from './auth.js';

// API types (chat, prompts, common patterns)
export type {
  Conversation,
  ConversationLastMessage,
  ConversationParticipant,
  ConversationParticipantRole,
  ConversationParticipantsResponse,
  Message,
  MessageActions,
  MessageRole,
  MessageFeedbackEntry,
  ActivityPillar,
  ApiMetadata,
  PaginatedResponse,
  ListResponse,
  CommandEntry,
  CommandCatalogueResponse,
} from './api.js';

// Chat turn envelope (the terminal document of one turn, on every surface)
export type {
  ChatMessageAction,
  ReplyVerdictChip,
  ReplyNotice,
  ReplyBlock,
  TurnTelemetry,
  AssistantTurn,
  TurnEnvelope,
  TurnProgress,
} from './turn.js';

// Claim-verdict row and the one severity rollup both clients read
export type {
  ClaimVerdictStatus,
  ClaimEvidenceStrength,
  ClaimVerdictCategory,
  ClaimVerdictLayer,
  VerdictTone,
  ClaimVerdict,
  VerdictSeverity,
  VerdictSummary,
} from './verdict.js';
export {
  VERDICT_STATUS_TONE,
  verdictChipSeverity,
  mergeVerdictSeverities,
  summarizeVerdicts,
  verdictSummaryLabel,
} from './verdict.js';

// Structured-workout plan types (builder-coach plan cards)
export type {
  WorkoutRange,
  WorkoutPlanWindow,
  WorkoutCompliance,
  WorkoutBlockType,
  WorkoutBlock,
  FuelingProtocol,
  FluidProtocol,
  WorkoutSession,
  WorkoutDayName,
  WorkoutDay,
  WorkoutWeek,
  WorkoutPlan,
} from './workout-plan.js';
export { parseWorkoutPlan } from './workout-plan.js';

// Notification types (push notifications, device tokens, preferences)
export type {
  NotificationCategory,
  DevicePlatform,
  DeviceToken,
  RegisterDeviceTokenRequest,
  NotificationPreferenceItem,
  NotificationPreferencesResponse,
  UpdateNotificationPreferenceRequest,
  NotificationActionType,
  NotificationAction,
  NotificationItem,
  NotificationFeedResponse,
  UnreadCountResponse,
  MarkAllReadResponse,
  ListNotificationsParams,
  BadgeSyncResponse,
} from './notifications.js';

// Admin types (API keys, admin tokens, A2A protocol, dashboard)
export type {
  ApiKeyStatus,
  ApiKey,
  ApiKeysResponse,
  CreateApiKeyRequest,
  CreateApiKeyResponse,
  AdminPermission,
  AdminToken,
  AdminTokensResponse,
  CreateAdminTokenRequest,
  CreateAdminTokenResponse,
  AdminTokenAudit,
  AdminTokenUsageStats,
  TierUsage,
  DashboardOverview,
  RateLimitOverview,
  RequestLog,
  RequestStats,
  RequestFilter,
  ToolUsageBreakdown,
  A2AClient,
  A2AClientRegistrationRequest,
  A2AClientCredentials,
  A2ASession,
  A2ARateLimitStatus,
  A2AUsageStats,
  A2AUsageRecord,
  A2ADashboardOverview,
  SetupStatusResponse,
  ProvisionedKey,
} from './admin.js';

// Group coaching types (groups, members, invites, analytics)
export type {
  GroupRole,
  GroupRespondMode,
  GroupInviteKind,
  OvertrainingRiskLevel,
  GroupTrend,
  SummaryDetailLevel,
  MemberFlag,
  HealthFlagSeverity,
  CoachingGroup,
  GroupMember,
  GroupInvite,
  UpdateGroupRequest,
  UpdateMemberRoleRequest,
  UpdatePeerConsentRequest,
  CreateInviteRequest,
  GroupAggregateStats,
  GroupHealthFlag,
  GroupWeeklyReport,
  GroupMembersResponse,
  GroupTranscriptEntry,
  GroupTranscriptResponse,
  TranscriptMember,
  GroupInvitesResponse,
  GroupStatsResponse,
  GroupWeeklyReportResponse,
  GroupHealthFlagsResponse,
  GroupPermissionsResponse,
} from './groups.js';

// Feature-flag types (GET /api/me/features)
export type {
  FeatureFlagMap,
  KnownFeatureFlag,
  MeFeaturesResponse,
} from './feature-flags.js';
