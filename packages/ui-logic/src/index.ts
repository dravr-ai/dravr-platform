// ABOUTME: Main entry point for @pierre/ui-logic package
// ABOUTME: Re-exports the headless UI hooks and the server-state hooks both clients bind

// Async action hook
export {
  type AsyncActionState,
  type UseAsyncActionProps,
  type UseAsyncActionReturn,
  useAsyncAction,
} from './useAsyncAction';

// A chat turn's trail of steps and its ticking clock, held for one thread
export {
  type StreamedReplyCallbacks,
  type TurnTrailState,
  streamedReplyCallbacks,
  useTurnTrail,
  useTurnElapsedMs,
} from './useTurnTrail';

// The composer's unsent line, kept per thread
export {
  type ComposerDraft,
  type ComposerDraftHandle,
  carryNoThreadDraft,
  useComposerDraft,
  writeComposerDraft,
} from './useComposerDraft';

// Toast hook
export {
  type ToastType,
  type ToastItem,
  type AddToastProps,
  type UseToastProps,
  type UseToastReturn,
  useToast,
} from './useToast';

// The password minimum and the e-mail shape every credential form checks
export { MIN_PASSWORD_LENGTH, isEmailAddress, isPasswordLongEnough } from './credentialRules';
export { signInTurnedDown } from './signInTurnedDown';

export { useAbortOnClose } from './useAbortOnClose';
export { isSignInPhaseLive, useSignInAttempt } from './useSignInAttempt';
export type { SignInAttempt, SignInAttemptOptions } from './useSignInAttempt';
export {
  type TakeoverResult,
  parseTakeoverBridgeMessage,
  parseTakeoverEvent,
  takeoverResult,
  takeoverTarget,
  takeoverTextChunks,
  takeoverTokenProvider,
} from './sciotteTakeover';
export {
  browserSigninLanded,
  hostOfHttps as browserSigninHost,
  hostUnder as browserSigninHostUnder,
} from './browserSignin';

// API error classification — one classifier for web and mobile, so a failed
// request is named by its transport state and status, never by matching on
// the server's own prose.
export {
  type ApiErrorKind,
  type ApiErrorTranslate,
  type ClassifiedApiError,
  classifyApiError,
  API_ERROR_KEYS,
  prefersServerDetail,
  describeApiError,
  describeQuotaRefusal,
  describeSignInFailure,
  type SignInFailure,
  refusalReason,
  INVALID_SIGNUP_CODE_REASON,
  isInvalidSignupCode,
  COACH_INVITE_NOT_FOR_ACCOUNT_REASON,
  isCoachInviteNotForAccount,
  refusalProvider,
  providerAuthRefusalKey,
  apiErrorStatus,
} from './apiError';
export { describeTurnFailure, isTurnFailureRetryable } from './turnFailure';

// Self-serve account deletion: the confirmation rule and the refusal wording.
export {
  type AccountDeletionFailure,
  emailConfirms,
  describeAccountDeletionBlocker,
  describeAccountDeletionFailure,
} from './accountDeletion';
// What the end of a downloaded data export says about the file.
export {
  EXPORT_PARTIAL_KEYS,
  EXPORT_CUT_SHORT,
  EXPORT_TAIL_BYTES,
  type ExportLimit,
  type ExportPartialKey,
  type ExportTail,
  exportPartialKey,
  exportRefusalCode,
  readExportTail,
  ORIGINALS_KEYS,
  ORIGINALS_POLL_MS,
  SEAL_MAX_CHARS,
  SEAL_MIN_CHARS,
  type OriginalsBundleState,
  type OriginalsKey,
  originalsIncomplete,
  originalsRefusalKey,
  originalsStateKey,
  originalsStillBuilding,
  sealPassphraseAccepted,
} from './dataExport';
// What both clients say about an import of the athlete's own data
export type { CopyLine, ImportPhase, ImportSourceCopy, LinkSourceCopy, UploadSourceCopy } from './archiveImport';
export {
  APPLE_HEALTH_COPY,
  AWAITING_LINK_LATE_MS,
  IMPORT_POLL_INTERVAL_MS,
  IMPORT_RESULT_STORAGE_KEY,
  importDate,
  importDeleteCopy,
  importFailureKey,
  importHistoryCopy,
  importOutcomeKey,
  importPhase,
  importSize,
  importSourceCopy,
  importStateCopy,
  importedHistoryCount,
  linkRefusalKey,
  offeredImportSources,
  startErrorKey,
  uploadErrorKey,
  uploadPercent,
} from './archiveImport';

export { cardAwaitingHistory, cardExportSource } from './cardHistory';
export {
  connectAction,
  followConnectPlan,
  type ConnectAction,
  type ConnectHandlers,
  type IntervalsDialog,
} from './connectDispatch';
export { coachAddDraft, coachMention } from './coachDraft';
export { orderTeam, teamInOrder, type OrderedTeam } from './team';

export type { ImportTombstoneRefusal } from './importTombstone';
export { describeRegistration } from './registration';
export {
  importTombstoneRefusal,
  importTombstoneFromReturn,
  describeProviderTombstone,
  describeEmailTombstone,
} from './importTombstone';

// Server-state hooks both clients bind to their own API instance. Each client
// keeps a thin `hooks/<name>` module that calls the factory once; only what is
// genuinely platform-specific (the API instance, a freshness the two clients
// set differently, how a key event is read) stays there.
export { type GroupFreshness, createGroupHooks } from './groupHooks';
// The coach-group onboarding sequence: group, thread, then the athlete invite, resumed on a retry.
export {
  type CoachGroupSetupApis,
  type UseCoachGroupSetupResult,
  createCoachGroupSetupHook,
} from './coachGroupSetupHook';
// A coach's plan thread for one athlete: opening it from Group info, and the draft card's Publish and Discard (carnet#1069/#1070),
// and the athlete's live plan read-only from Group info (carnet#1244).
export {
  type AthletePlanRead,
  type CoachPlanApis,
  type OpenPlanDraft,
  type PlanThreadDraft,
  type PlanDraftActionResult,
  type PlanDraftCardState,
  type SubjectPlanRead,
  type UseAthletePlanResult,
  type UseOpenAthleteThreadResult,
  type UseOpenGroupRoomResult,
  type UsePlanDraftCardResult,
  createCoachPlanHooks,
  openThreadRefusalText,
} from './coachPlanHooks';
// A coach's athlete workspace: the fortnight as seven dates a week, each plan change in words,
// and the athlete's room and plan thread among what the coach holds (carnet#1816)
export {
  type PlanCalendarDay,
  type PlanCalendarWeek,
  type PlanDateTiming,
  type PlanHistoryLine,
  type PlanHistoryWords,
  PLAN_WORKSPACE_KEYS,
  athleteRoomGroup,
  canOpenWorkspace,
  planCalendarWeeks,
  planHistoryLine,
  roomConversationOf,
  shownPlan,
  usablePlanThread,
} from './planWorkspace';
// The athlete workspace's two threads, the coach's answers to the draft and which state its plan and tabs are in, one definition both clients bind (carnet#1816, #1818)
export {
  type WorkspaceAnswers,
  type WorkspaceGroupTab,
  type WorkspacePlanRead,
  type WorkspaceRoom,
  type WorkspaceThreads,
  createWorkspaceHooks,
  workspaceAgentName,
  workspaceGroupTab,
  workspacePlanState,
} from './workspaceHooks';
// A coach's Groups list with one row per trifecta athlete, opening their workspace (carnet#1817)
export { type AthleteWorkspaceFold, type GroupsListRow, foldAthleteRows } from './athleteRows';
// A coach's day editor: the day's own session by hand, or a family build with its live preview, saved into the draft (carnet#1515, #1714)
export {
  type DayReplacement,
  type DerivedRow,
  type EditorMode,
  type HandSession,
  type MissingLine,
  type SportPick,
  DATE_FULL_REASON,
  MAX_SESSIONS_PER_DATE,
  SESSION_MOVED_REASON,
  type SessionAt,
  type SessionEditorApis,
  SESSION_EDITOR_KEYS,
  SESSION_PARAM_LABEL_KEY,
  SESSION_PREVIEW_DEBOUNCE_MS,
  TARGET_ANCHOR_LABEL_KEY,
  addSeries,
  addStep,
  addStepToSeries,
  canAddSession,
  canRemoveSession,
  createSessionEditorHooks,
  dayIsUncheckedEdit,
  dayPosition,
  derivedRows,
  duplicateStep,
  editableParams,
  editorDirty,
  familiesForSport,
  familyBuildChanged,
  familyOf,
  formatStepDuration,
  handDayRequest,
  handSessionChanged,
  handSessionComplete,
  handSessionMissing,
  handSessionOf,
  handStatesDuration,
  handSummary,
  initialEditorMode,
  initialSessionBuild,
  isDateFull,
  isSessionMoved,
  flaggedParams,
  moveStep,
  newSessionDay,
  offeredFamilySport,
  paramValue,
  parseStepDuration,
  pickSport,
  presetOf,
  presetsForSport,
  removeStep,
  replacementLine,
  restoreLine,
  selectBuildSport,
  sessionExpect,
  selectFamily,
  selectPreset,
  selectSport,
  seriesDefaults,
  setHand,
  setParam,
  setSeriesRepeat,
  setStepMeasureOf,
  unresolvedTargetLine,
  updateStep,
} from './sessionEditor';
// The structure of a hand-written session: sets under one ×N, time or distance, targets the chips compose (carnet#1727)
export {
  type ProfileBar,
  type ProfileBlock,
  type SeriesDefaults,
  type StepBlock,
  type StepMeasure,
  type StepTarget,
  type TargetAnchor,
  MAX_TARGET_ZONE,
  PROFILE_EFFORT_LEVEL,
  SERIES_DEFAULT_REPEAT,
  SERIES_MAX_REPEAT,
  SERIES_MIN_REPEAT,
  TARGET_ANCHORS,
  defaultAnchor,
  parseTarget,
  percentTarget,
  profileBlocks,
  rpeTarget,
  stepBlocks,
  stepMeasure,
  stepMoveTarget,
  stepsTotalSeconds,
  targetSpan,
  zoneTarget,
} from './sessionSteps';
// The sports a coach picks for a session: one-tap chips, the day's own word, a sport they name (carnet#1727)
export {
  type QuickSport,
  QUICK_SPORTS,
  chosenSport,
  familySportFor,
  quickSportOf,
  sameSport,
  sportChoices,
} from './sessionSports';
// A coach's private lane in a one-athlete room: the Everyone / Only <agent> switch, the work
// thread's rows drawn in the room, "Adjust plan from this" and the private send (carnet#1249).
export {
  type AudienceStore,
  type PrivateLaneApis,
  type PrivateSendOptions,
  type PrivateSendOutcome,
  type UsePrivateLaneInput,
  type UsePrivateLaneResult,
  createPrivateLaneHooks,
  privateSendFailureText,
} from './privateLaneHooks';
export {
  type UnreadCountPolling,
  createNotificationHooks,
  invalidateNotificationReads,
} from './notificationHooks';
export { type UseFeatureFlagsResult, createFeatureFlagsHook } from './featureFlagsHook';
export { type UseHomePreferencesResult, createHomePreferencesHook } from './homePreferencesHook';
// Consent to AI use of one provider's data: the privacy toggle and the connection card share it (carnet#726).
export {
  type AiConsentChange,
  type TransferConsentChange,
  createAiConsentHook,
  createAiConsentStateHook,
  createTransferConsentHook,
} from './aiConsentHook';
// The provider cards on the one provider-status key: every screen that connects or disconnects reads them here.
export { type ProviderCards, type ProviderCardsOptions, createProviderCardsHook } from './providerCardsHook';
// The "Who read your data" feed, and the one Connected-apps key and revoke it shares with both Connected-apps screens (carnet#1543).
export {
  type AccessFeedApiKeyGroup,
  type AccessFeedAppGroup,
  createAccessFeedHook,
  createConnectedAppsHook,
  createRevokeConnectedAppsHook,
  groupAccessFeedApiKeyDays,
  groupAccessFeedApps,
  revokedInKeySettings,
} from './accessFeedHook';
// A Connected app's signed grant receipts and the athlete's standing policies (carnet#1544).
export {
  createConnectedAppReceiptsHook,
  createGrantReceiptsHook,
  createRemoveStandingPolicyHook,
  createStandingPoliciesHook,
  GRANT_RECEIPT_PAGE,
} from './grantHistoryHooks';
// Statutory access requests to a company: the letter the athlete sends and its legal deadline (carnet#1646).
export {
  type DataAccessAction,
  type DataAccessDateLine,
  type DataAccessDraftErrors,
  type DataAccessDraftForm,
  type DataAccessOutcome,
  type DataAccessRequestHooks,
  type DataAccessTone,
  DATA_ACCESS_COMPANY_SUGGESTIONS,
  DATA_ACCESS_FIELD_LIMITS,
  DATA_ACCESS_LAW_OPTIONS,
  DATA_ACCESS_RECOURSE_KEY,
  DATA_ACCESS_SCOPE_OPTIONS,
  addCivilDays,
  addCivilMonths,
  createDataAccessRequestHooks,
  dataAccessDateLines,
  dataAccessDraftBody,
  dataAccessFailureKey,
  dataAccessShowsRecourse,
  dataAccessStatus,
  dataAccessToday,
  defaultLetterLocale,
  earliestSentOn,
  gdprExtensionWindow,
  isCivilDate,
  validateAnsweredOn,
  validateDataAccessDraft,
  validateExtension,
  validateSentOn,
} from './dataAccessRequests';
// The calendar Dravr's plan and workout pushes go to: Connections settings and the agent write it (carnet#1393).
export {
  type UsePreferredCalendarResult,
  createPreferredCalendarHook,
  hasCalendarChoice,
} from './preferredCalendarHook';
// How often the coach checks in on its own: the onboarding step and Settings write it (carnet#831).
export { type UseCheckInResult, CHECK_IN_OPTIONS, createCheckInHook } from './checkInHook';
// The athlete's units, metric unless Settings chose imperial, and the context every distance a surface prints reads (carnet#835).
export {
  type UseUnitPreferencesResult,
  UNIT_SYSTEM_OPTIONS,
  UnitsContext,
  createUnitPreferencesHook,
  useDistanceUnit,
} from './unitsHook';
// Settings › Physiology: the read of the athlete's anchors and the facts under each one (carnet#1516).
export { type PhysiologyPlatform, createPhysiologyHook, physiologyAnchorFacts } from './physiology';
// Setting one anchor, the athlete's own or a coached athlete's from Group info (carnet#1561).
export {
  type AnchorForm,
  type AnchorFormProblem,
  type AnchorFormRead,
  type AnchorFormTarget,
  type AnchorSave,
  type AthletePhysiologyRead,
  type UseAthletePhysiologyResult,
  anchorEntryUnit,
  createCoachPhysiologyHooks,
  createPhysiologyEditHook,
  initialAnchorForm,
  newAnchorForm,
  paceDistanceFor,
  readAnchorForm,
} from './physiologyEdit';
export {
  type ActivityUploadFailure,
  type ActivityUploadOutcome,
  type PickedActivityFile,
  type UseActivityUploadResult,
  type UseDeleteUploadedActivityResult,
  ACTIVITY_DELETE_KEYS,
  ACTIVITY_UPLOAD_KEYS,
  ACTIVITY_UPLOAD_MAX_MEGABYTES,
  activityUploadFailure,
  createActivityUploadHook,
  createDeleteUploadedActivityHook,
  describeActivityUpload,
  isDeletableActivity,
} from './activityUploadHook';
export {
  type HealthRecordsHooks,
  type LabUploadOutcome,
  type PickedLabDocument,
  type ReviewAnswer,
  type UseLabDocumentUploadResult,
  HEALTH_RECORDS_KEYS,
  HEALTH_RECORD_KIND_LABEL_KEY,
  checkLabel,
  createHealthRecordsHooks,
  describeLabUpload,
  labUploadOutcome,
  observationName,
  observationRange,
  observationValue,
  selfReportKindLabel,
  selfReportValue,
  selfReportValueLabel,
} from './healthRecordsHook';

// Composer palettes. Platform-free: each composer reads its own key event and
// hands the palette the key's name.
export { PALETTE_KEYS } from './paletteKeys';
export {
  type UseCommandPaletteOptions,
  type UseCommandPaletteResult,
  createCommandPaletteHook,
} from './commandPalette';
export {
  type UseMentionPaletteOptions,
  type UseMentionPaletteResult,
  createMentionPaletteHook,
} from './mentionPalette';

// Home's reads: recent activities and their stale-answer schedule, a route, the plan, status and volume.
export {
  type ActivityRouteOptions,
  type ActivityRouteState,
  type HomeReadApi,
  type HomeReadPlatform,
  type RecentActivitiesState,
  type StaleRefetchPhase,
  createHomeReadHooks,
} from './homeReadHooks';

// One activity's view: its figures, its split and lap tables, the questions it offers.
export {
  type ActivityFigure,
  type ActivityPrompt,
  type SegmentRow,
  type SegmentTable,
  type SpeedForm,
  ACTIVITY_PROMPTS,
  type AskAboutTranslate,
  activityFigures,
  activityFirstLine,
  formatClock,
  formatActivityDistance,
  formatSpeed,
  lapsTable,
  speedForm,
  splitsTable,
} from './activityView';
// Home's words and figures: calendar days, instants, sports, row figures and the plan drafts, one rule on both clients.
export {
  type PlanWindow,
  type Translate,
  DRAFT_DATE,
  ROW_DATE,
  activityNaming,
  activityRowFigures,
  activityStartLine,
  calendarDayLabel,
  civilDayMonth,
  civilDayOfMonth,
  civilMonthTitle,
  civilShortDate,
  civilWeekdayNarrow,
  formatActivityDay,
  formatCivilDate,
  formatInstant,
  formatSyncTime,
  phaseWeekLabel,
  planDayDraft,
  planDayRouteDraft,
  planDayMinutes,
  planWindow,
  sessionFacts,
  sportLabel,
  volumeMetricText,
  volumeReadoutShown,
} from './homeFormat';
export { type ActivityDetailState, createActivityDetailHook } from './activityDetailHook';
export { type ActivityConversationState, createActivityConversationHook } from './activityConversation';

// An agent system prompt's estimated size, worded by one catalogue sentence on both clients.
export { PROMPT_TOKEN_ESTIMATE_KEY, estimatePromptTokens } from './promptTokens';

// Home activity list: whether it is fetching, failed or settled, said once on both clients.
export {
  type RecentActivitiesSync,
  type RecentActivitiesSyncInput,
  type RequestsInFlight,
  recentActivitiesSync,
  useRequestsInFlight,
} from './recentActivitiesSync';

// Home's volume-card sport, which the activity list beside it filters on too.
export { type VolumeSportFilter, useVolumeSportFilter } from './volumeSportFilter';

// The gear ledger: the athlete's shoes, their distance, and the pair each activity was run in.
export {
  type ActivityGearState,
  type GearChanges,
  type GearDraft,
  type GearDraftCheck,
  type GearDraftErrors,
  type GearHooks,
  checkGearDraft,
  createGearHooks,
  emptyGearDraft,
  formatGearDistance,
  gearChanges,
  gearChoices,
  gearDistanceLimit,
  gearDraftOf,
  splitGear,
} from './gearLedger';

// Coach Home: the queue, the team table's order, the season's layout and the words of an evidence line, one rule on both clients.
export {
  FORM_STRIP_STEPS,
  agentCell,
  agentCellNeedsLook,
  athletesInGroup,
  buildQueue,
  buildRosterTable,
  buildSeason,
  civilDayNumber,
  civilFromDayNumber,
  countdownBars,
  cutReadingDay,
  daysBetween,
  daysToARace,
  describeEvidence,
  describeLine,
  describeReasonTitle,
  describeSource,
  endedPlan,
  firstQueued,
  formStripIndex,
  localToday,
  nextPhase,
  planLastDay,
  providerLetter,
  queueCount,
  queueWhen,
  reasonTitle,
  restatesReason,
  sourceChips,
  trainingCut,
  type CoachQueue,
  type EndedPlan,
  type EvidenceLineText,
  type MergedRoster,
  type QueueEntry,
  type ReasonTitle,
  type SeasonRow,
  type SeasonScale,
} from './coachHome';
export { createCoachRosterHooks } from './coachRosterHooks';
// The coach's morning brief: its status line, hours and lists, and the hooks over its endpoints (carnet#1791).
export {
  BRIEF_HOURS,
  BRIEF_REASON_KINDS,
  applyBriefUpdate,
  briefHourLabel,
  briefStatusLine,
  briefStatusWords,
  isBriefMuted,
  isCoachBrief,
  mutedAthletes,
  withMuted,
  withReason,
  type BriefStatusWords,
} from './coachBrief';
export { createCoachBriefHooks, type CoachBriefHooks, type UseCoachBriefResult } from './coachBriefHooks';
// A coach-access grant picked up without signing out, and the list's coach-seat line (carnet#1521).
export {
  createCoachAccessHooks,
  seatLineDay,
  type CoachAccessViewer,
  type SeatProbe,
  type SeatProbeOptions,
} from './coachAccessHooks';
// Coach Home's invited-but-not-joined rows (carnet#1524).
export { COACH_PENDING_INVITE_KEYS, groupsAwaitingAthlete, usableMemberInvite } from './coachPendingInvites';
export {
  inviteCardNamesGroupBelow,
  inviteCardTitle,
  inviteDeadLink,
  inviteJourneyLine,
  inviteJourneyPromise,
  inviteShares,
  joinStripFull,
  joinStripJoin,
  joinStripTitle,
  type InviteCopyLine,
  type InvitePeople,
} from './inviteCopy';
export {
  INVITE_FUNNEL_KEYS,
  inviteFunnelSteps,
  type InviteFunnelStep,
  type InviteFunnelStepKey,
  type InviteFunnelStepState,
} from './inviteFunnel';
export {
  ALL_COACHED_ATHLETES,
  coachedAthleteOptions,
  replyStatus,
  type CoachedAthleteOption,
  type ReplyStatus,
} from './coachReplies';
export {
  agentWords,
  formWords,
  lastSessionWords,
  lastWhenWords,
  nonSharerStatusWords,
  planWords,
  sourceChipWords,
  threadWords,
} from './coachHomeWords';
