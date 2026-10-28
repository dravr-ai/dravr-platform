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
  TeamRule,
  TeamMember,
  ProposedTeam,
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
  SignupConfig,
  EmailConfirmationStatus,
  EmailConfirmationResponse,
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
  ImportTombstoneKind,
  EmailTombstone,
  ClearedImportTombstone,
  ClearImportTombstoneResponse,
  RedeemSignupCodeResponse,
} from './auth.js';
export { IMPORT_TOMBSTONED_ERROR } from './auth.js';

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
  UseCaseStarter,
  StartersResponse,
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
  AwaitingReply,
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
  VERDICT_STATUS_ORDER,
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

// An RFC 3339 instant as epoch milliseconds, read the same in the browser and on Hermes
export { instantMs } from './instant.js';

// A coach's plan draft for one athlete: its state, the subject-plan read, publish and its conflict
export type {
  DraftConflictReason,
  DraftConflict,
  DraftState,
  PlanSubject,
  SubjectPlanResponse,
  PublishSubjectPlanRequest,
  PublishedSubjectPlan,
  PlanPublishConflict,
  PublishSubjectPlanResult,
} from './coach-plan.js';
export {
  parseDraftState,
  parsePlanSubject,
  parseSubjectPlanResponse,
  parsePublishedSubjectPlan,
  parsePlanPublishConflict,
} from './coach-plan.js';

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
  HomePreferencesUpdate,
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
  DEFAULT_HOME_PREFERENCES,
  parseCalendarResponse,
} from './home.js';

// A completed workout's .fit file uploaded by the athlete: the Home rows it became
export type { ActivityUploadResponse, HeldActivity } from './activity-upload.js';
export { ACTIVITY_UPLOAD_MAX_BYTES, UPLOAD_PROVIDER, parseActivityUploadResponse } from './activity-upload.js';
export type {
  ArchiveImport,
  ImportFailureCode,
  ImportIssue,
  ImportLinkCheck,
  ImportLinkRefusal,
  ImportReportPage,
  ImportStatus,
  ImportsOverview,
} from './imports.js';
export {
  STRAVA_ARCHIVE_SOURCE,
  isFinalImportStatus,
  linkRefusalOf,
  parseArchiveImport,
  parseCurrentImport,
  parseImportLinkCheck,
  parseImportReportPage,
  parseImportsOverview,
} from './imports.js';

// The athlete's gear ledger: the shoes they declare, their distance, the pair one activity was run in
export type {
  ActivityGearResponse,
  CreateGearRequest,
  GearAttributionSource,
  GearItem,
  GearKind,
  GearListResponse,
  GearSport,
  UpdateGearRequest,
} from './gear.js';
export {
  GEAR_DEFAULT_WARN_DISTANCE_METERS,
  GEAR_MAX_DISTANCE_METERS,
  GEAR_NAME_MAX_CHARS,
  GEAR_SPORTS,
  isGearSport,
  parseActivityGearResponse,
  parseGearItem,
  parseGearListResponse,
} from './gear.js';

// The athlete Home page's weekly volume: distance, time and climbing per sport, week by week
export type { SportVolume, TrainingVolumeResponse, VolumeWeek } from './training-volume.js';
export { parseTrainingVolumeResponse } from './training-volume.js';

// What Garmin-sourced data contributed to a Home figure, which decides the line beneath its title (carnet#1154)
export type { GarminContribution } from './garmin-attribution.js';

// The athlete's in-app feedback thread with the Dravr team (carnet#1235)
export type {
  FeedbackAbout,
  FeedbackMessage,
  FeedbackThreadResponse,
  SendFeedbackRequest,
} from './feedback.js';
export { FEEDBACK_MESSAGE_MAX_CHARS, parseFeedbackCreated, parseFeedbackThread } from './feedback.js';

// How often the coach checks in on its own: daily, three a week, weekly or off (carnet#831)
export type { CheckInCadence, CheckInPreference } from './check-in.js';
export { CHECK_IN_CADENCES, isCheckInCadence, parseCheckInPreference } from './check-in.js';

// Home's Getting started list: connect, then the first-run flows (carnet#1099)
export type { GettingStartedResponse, GettingStartedRow } from './getting-started.js';
export { parseGettingStartedResponse } from './getting-started.js';

// The athlete's units: metric unless they chose imperial in Settings (carnet#835, carnet#1044)
export type { UnitPreferences, UnitSystem } from './units.js';
export { parseUnitPreferences } from './units.js';

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
} from './notifications.js';

// Admin types (admin tokens, A2A protocol)
export type {
  AdminPermission,
  AdminToken,
  AdminTokensResponse,
  CreateAdminTokenRequest,
  CreateAdminTokenResponse,
  RotateAdminTokenResponse,
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
  CoachSharing,
  CreateGroupRequest,
  CreateInviteRequest,
  GroupAggregateStats,
  GroupHealthFlag,
  FlagEvidence,
  FreshMember,
  GroupWeeklyReport,
  GroupMembersResponse,
  GroupTranscriptEntry,
  CorrectedReply,
  GroupTranscriptPage,
  GroupTranscriptResponse,
  TranscriptMember,
  TranscriptSpeaker,
  GroupInvitesResponse,
  GroupStatsResponse,
  GroupWeeklyReportResponse,
  GroupHealthFlagsResponse,
  GroupAnalyticsScope,
  GroupPermissionsResponse,
  DelegationStatus,
  DelegationViewer,
  DelegatedConnection,
  DelegatedConnectionsResponse,
  CoachPlatformProvider,
  DelegationRosterAthlete,
  DelegationRosterResponse,
  ProposeDelegatedConnectionRequest,
  ConfirmDelegatedConnectionRequest,
  CoachConsentsResponse,
  UpdateCoachConsentsRequest,
  DelegationRefusalReason,
  DelegationReadRefusal,
  WellnessReadSkip,
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

export { MEMORY_FACT_KINDS, MEMORY_FACT_STATUSES } from './memory';
export type { MemoryEvidenceSource, MemoryFactKind, MemoryFactStatus } from './memory';

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
  ApiKeyToolUsage,
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

// A coach's after-the-fact review of the agent's group replies (carnet#847)
export type {
  ReplyFlagReason,
  CoachReplyFlag,
  CoachReplyCorrection,
  CoachReply,
  CoachedGroup,
  CoachedAthlete,
  CoachRepliesPage,
  CoachRepliesQuery,
  FlagReplyResponse,
  CorrectReplyResponse,
  CoachReviewWeek,
  CoachReviewWeeklyResponse,
} from './coach-review';
