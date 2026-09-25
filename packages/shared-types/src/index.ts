// ABOUTME: Main entry point for @pierre/shared-types package
// ABOUTME: Re-exports all shared types for convenient importing

// Coach types (agent personas, store, versions)
export type {
  ActivityDataRequirements,
  DataRequirements,
  AgentCategory,
  AgentVisibility,
  PublishStatus,
  Agent,
  UpdateAgentRequest,
  AgentMetadata,
  ListAgentsResponse,
  StoreAgent,
  StoreAgentDetail,
  StoreMetadata,
  BrowseAgentsResponse,
  SearchAgentsResponse,
  InstallAgentResponse,
  UninstallAgentResponse,
  InstallationsResponse,
  AgentAssignment,
  AssignAgentResponse,
  UnassignAgentResponse,
  ListAssignmentsResponse,
  SportShare,
  SportProfileSummary,
  ProposedAgent,
  AgentProposalResponse,
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
  ProviderDelegation,
  SciotteTarget,
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
  VerdictDisposition,
  DispositionReason,
  VerdictTone,
  ClaimVerdict,
  VerdictSeverity,
  VerdictSummary,
} from './verdict.js';
export {
  CLAIM_VERDICT_LAYERS,
  VERDICT_DISPOSITIONS,
  DISPOSITION_REASONS,
  VERDICT_STATUS_TONE,
  verdictChipSeverity,
  verdictToneAlerts,
  summarizeVerdicts,
} from './verdict.js';

// The workout_plan block: the saved plan projected for a card
export type {
  PlanPhaseKind,
  PlanSelectedBy,
  PlanTemplateSource,
  PlanGoalRace,
  PlanFlavour,
  PlanPhase,
  PlanStep,
  PlanFueling,
  PlanDay,
  PlanWeek,
  WorkoutPlan,
} from './workout-plan.js';
export { parseWorkoutPlan } from './workout-plan.js';

// Civil-date reads over a plan card: session, rest day, or a date the plan does not cover
export type { PlanDayLookup } from './plan-calendar.js';
export { addCivilDays, mondayOf, planDayOn, phaseWeekOn } from './plan-calendar.js';

// The athlete Home page's reads: recent activities, one activity's route, the plan for today
export type {
  HomeActivity,
  RecentActivitiesResponse,
  ActivityRouteUnavailableReason,
  ActivityRouteResponse,
  TrainingPlanResponse,
} from './home.js';
export {
  ACTIVITY_ROUTE_UNAVAILABLE_REASONS,
  parseRecentActivitiesResponse,
  parseActivityRouteResponse,
  parseTrainingPlanResponse,
} from './home.js';

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

// Admin types (admin tokens, A2A protocol, dashboard)
export type {
  AdminPermission,
  AdminToken,
  AdminTokensResponse,
  CreateAdminTokenRequest,
  CreateAdminTokenResponse,
  ToolUsageBreakdown,
  A2AClient,
  A2AClientRegistrationRequest,
  A2AClientCredentials,
  A2ARateLimitStatus,
  A2AUsageStats,
} from './admin.js';

// Group coaching types (groups, members, invites, analytics)
export type {
  GroupRole,
  GroupRespondMode,
  GroupDigestMode,
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
  FlagEvidence,
  FreshMember,
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
  DelegationStatus,
  DelegationViewer,
  DelegatedConnection,
  DelegatedConnectionsResponse,
  DelegationRosterAthlete,
  DelegationRosterResponse,
  ProposeDelegatedConnectionRequest,
  DelegationRefusalReason,
} from './groups.js';

// Feature-flag types (GET /api/me/features)
export type {
  FeatureFlagMap,
  KnownFeatureFlag,
  MeFeaturesResponse,
} from './feature-flags.js';

// The live string catalogue a client overlays on its embedded copy
export type { I18nBundle, I18nBundleResult } from './i18n';

export type { PersonaCard, PersonaRule, PersonasResponse } from './personas';

export { MEMORY_FACT_KINDS } from './memory';
export type { MemoryFactKind } from './memory';

// The quota counters GET /api/usage/status serves
export type { LimitCheckResult, UsageStatusResponse } from './usage';

// Billing: subscription, invoices, plan catalogue, quota snapshot, checkout and portal
export type {
  PlanTier,
  PaidPlanTier,
  SubscriptionView,
  BillingInvoice,
  InvoicesResponse,
  QuotaCounter,
  MyQuotaResponse,
  PlanView,
  PlansResponse,
  CheckoutRequest,
  CheckoutResponse,
  PortalRequest,
  PortalResponse,
} from './billing';
