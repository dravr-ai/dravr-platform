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
  AgentContentSnapshot,
  AgentVersion,
  ListAgentVersionsResponse,
  AgentFieldChange,
  AgentVersionDiffResponse,
  RevertAgentVersionResponse,
  SubmitAgentForReviewResponse,
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
  SciotteLoginResponse,
  AccountDeletionBlockerKind,
  AccountDeletionBlocker,
  AccountDeletionPreview,
  DeleteAccountRequest,
  DeleteAccountResponse,
  AccountDeletionRefusal,
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
  RoomAttribution,
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
  MessageOrigin,
  TurnSendOptions,
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
  EvidenceCitation,
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
export {
  addCivilDays,
  mondayOf,
  planDayOn,
  planWeeksDayOn,
  planDayDistanceMeters,
  phaseWeekOn,
} from './plan-calendar.js';

// Home's calendar: the strip's weeks, a month's grid, how far it pages, one day's entries
export type { CalendarDay } from './home-calendar.js';
export {
  activityMinutes,
  calendarDay,
  firstOfMonth,
  lastMonday,
  monthGrid,
  shiftMonth,
  weekDays,
} from './home-calendar.js';

// The athlete Home page's reads: recent activities, one activity's view and route, the plan for today
export type {
  ActivityDetailResponse,
  ActivityLap,
  ActivitySplit,
  HomeActivity,
  RecentActivitiesResponse,
  SyncFailure,
  ActivityRouteUnavailableReason,
  ActivityRouteAnswer,
  ActivityRoutePending,
  ActivityRouteResponse,
  TrainingPlanResponse,
  FormBand,
  FormReading,
  FormTrendPoint,
  TrainingLoadRatio,
  TrainingStatusResponse,
  HomePreferences,
  CalendarActivity,
  CalendarResponse,
} from './home.js';
export {
  ACTIVITY_ROUTE_UNAVAILABLE_REASONS,
  parseActivityDetailResponse,
  parseRecentActivitiesResponse,
  parseActivityRouteResponse,
  parseTrainingPlanResponse,
  FORM_BANDS,
  parseTrainingStatusResponse,
  parseHomePreferences,
  parseCalendarResponse,
} from './home.js';

// The athlete Home page's weekly volume: distance, time and climbing per sport, week by week
export type { SportVolume, TrainingVolumeResponse, VolumeWeek } from './training-volume.js';
export { parseTrainingVolumeResponse } from './training-volume.js';

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
  RotateAdminTokenResponse,
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
  CreateGroupRequest,
  CreateInviteRequest,
  GroupAggregateStats,
  GroupHealthFlag,
  FlagEvidence,
  FreshMember,
  GroupWeeklyReport,
  GroupMembersResponse,
  GroupTranscriptEntry,
  GroupTranscriptPage,
  GroupTranscriptResponse,
  TranscriptMember,
  TranscriptSpeaker,
  GroupInvitesResponse,
  GroupStatsResponse,
  GroupWeeklyReportResponse,
  GroupHealthFlagsResponse,
  GroupPermissionsResponse,
  DelegationStatus,
  DelegationViewer,
  DelegatedConnection,
  DelegatedConnectionsResponse,
  CoachPlatformProvider,
  DelegationRosterAthlete,
  DelegationRosterResponse,
  ProposeDelegatedConnectionRequest,
  DelegationRefusalReason,
  DelegationReadRefusal,
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

// The athlete's own API keys, as /api/keys serves them
export type {
  ApiKeyTier,
  ApiKeyInfo,
  ApiKeyListResponse,
  CreateApiKeyRequest,
  ApiKeyCreateResponse,
  ApiKeyDeactivateResponse,
  ApiKeyUsageStats,
  ApiKeyUsageResponse,
} from './api-keys';

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

// Coach-access requests: a coach asks, a super-admin grants or declines (carnet#738)
export type {
  CoachAccessStatus,
  CoachAccessRequest,
  CoachAccessRequestResponse,
  CoachAccessRequestView,
} from './coach-access';
