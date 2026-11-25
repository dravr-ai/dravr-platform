// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Centralized query key constants for React Query
// ABOUTME: Ensures type safety and consistency across all query/mutation operations

/**
 * Centralized query keys for React Query.
 *
 * Benefits:
 * - Type-safe query key management
 * - Easy refactoring - change keys in one place
 * - Prevents typos and inconsistencies
 * - Better IDE autocomplete support
 *
 * Usage:
 * ```typescript
 * import { QUERY_KEYS } from '@pierre/shared-constants';
 *
 * // In useQuery
 * useQuery({ queryKey: QUERY_KEYS.coaches.list() })
 *
 * // In invalidateQueries
 * queryClient.invalidateQueries({ queryKey: QUERY_KEYS.coaches.all })
 * ```
 */

export const QUERY_KEYS = {
  // ==================== AUTH & USER ====================
  user: {
    all: ['user'] as const,
    profile: () => ['user-profile'] as const,
    stats: () => ['userStats'] as const,
    oauthApps: () => ['user-oauth-apps'] as const,
    onboardingStatus: () => ['user-onboarding-status'] as const,
    coachAccessRequest: () => ['user-coach-access-request'] as const,
    /** The units the athlete reads, and the Settings choice behind them. */
    units: () => ['user-units'] as const,
    /**
     * The athlete's anchors per sport family (carnet#1516). The server writes
     * their words in the app's language and the athlete's units, so both are
     * in the key: a language or units switch reads them again.
     */
    physiology: (locale: string, units: string) => ['user-physiology', locale, units] as const,
    /** How often the coach checks in on its own (carnet#831). */
    checkIn: () => ['user-check-in'] as const,
    /** The calendar Dravr's plan and workout pushes go to (carnet#1393). */
    preferredCalendar: () => ['user-preferred-calendar'] as const,
    /** The athlete's feedback thread with the Dravr team (carnet#1235). */
    feedbackThread: () => ['user-feedback-thread'] as const,
    /** The team's replies in that thread the athlete has not opened (carnet#1392). */
    feedbackUnread: () => ['user-feedback-unread'] as const,
    /** The screenshot one of the athlete's own messages in that thread carries (carnet#1392). */
    feedbackImage: (messageId: string) => ['user-feedback-image', messageId] as const,
  },

  // ==================== SIGN-UP ====================
  signup: {
    /** Whether sign-up is open and which bot-check site key the form renders (public). */
    config: () => ['signup-config'] as const,
  },

  // ==================== ACCOUNT ====================
  account: {
    /** What deleting the account would remove, read before the athlete confirms. */
    deletionPreview: () => ['account-deletion-preview'] as const,
    /**
     * The athlete's latest originals bundle. Every answer about a bundle (the
     * request's and each poll's) is written here, so a pane opened again shows it.
     */
    originalsLatest: () => ['originals-latest'] as const,
    /** One originals bundle by id, polled while it is still being built. */
    originalsBundle: (bundleId?: string) => ['originals-bundle', bundleId] as const,
  },

  // ==================== HEALTH RECORDS ====================
  healthRecords: {
    /** Every key below; invalidated after any answer, upload or delete, since each moves records between them. */
    all: ['health-records'] as const,
    /** The served records of one kind. */
    records: (kind: string) => ['health-records', 'records', kind] as const,
    /** The records read from a lab report that wait for the athlete's answer. */
    review: () => ['health-records', 'review'] as const,
    /** The athlete's uploaded lab reports. */
    documents: () => ['health-records', 'documents'] as const,
  },

  // ==================== DATA ACCESS REQUESTS ====================
  dataAccessRequests: {
    /** Every key below; invalidated after any write, since a step moves a request's state and letter. */
    all: ['data-access-requests'] as const,
    /** The athlete's requests, newest first. */
    list: () => ['data-access-requests', 'list'] as const,
    /** Every language of one request's letter, dropped when a step changes the request. */
    letters: (id: string) => ['data-access-requests', 'letter', id] as const,
    /** One request's letter, in one language. */
    letter: (id: string, locale: string) => ['data-access-requests', 'letter', id, locale] as const,
  },

  // ==================== OAUTH ====================
  oauth: {
    all: ['oauth'] as const,
    status: () => ['oauth-status'] as const,
    connections: () => ['connections'] as const,
    /**
     * The outside apps the athlete approved on the OAuth consent screen
     * (`GET /api/me/oauth-grants`): read by both Connected-apps screens and
     * the access feed's Revoke, and reread after every revoke.
     */
    connectedApps: () => ['oauth', 'connected-apps'] as const,
    /** One connected app grant's receipts (`GET /api/me/oauth-grants/{id}/receipts`). */
    connectedAppReceipts: (grantId: string) => ['oauth', 'connected-apps', grantId, 'receipts'] as const,
    /** Every receipt of the athlete's grants (`GET /api/me/oauth-grant-receipts`), one page. */
    grantReceipts: (limit: number, offset: number) => ['oauth', 'grant-receipts', limit, offset] as const,
    /** The athlete's standing policies (`GET /api/me/oauth-standing-policies`), reread after a removal. */
    standingPolicies: () => ['oauth', 'standing-policies'] as const,
  },

  // ==================== ACCESS FEED ====================
  accessFeed: {
    /** The athlete's "Who read your data" feed (carnet#1543), reread after a Connected-apps revoke. */
    feed: () => ['access-feed'] as const,
  },

  // ==================== PROVIDERS ====================
  providers: {
    all: ['providers'] as const,
    status: () => ['providers-status'] as const,
    /** The account's answer to one consent to AI use (`GET …/ai-consent`). */
    aiConsent: (provider: string) => ['providers-ai-consent', provider] as const,
  },

  // ==================== HOME ====================
  home: {
    all: ['home'] as const,
    /**
     * The newest cached activities; `limit` omitted is the server's default of
     * 5, `sport` omitted is every sport.
     */
    recentActivities: (limit?: number, sport?: string | null) =>
      ['home', 'recent-activities', limit ?? null, sport ?? null] as const,
    /**
     * One activity's route. Keyed by provider as well as id, because an id is
     * only unique within the provider that issued it.
     */
    activityRoute: (provider: string, activityId: string) =>
      ['home', 'activity-route', provider, activityId] as const,
    /** One workout's own view: its figures, splits and laps. */
    activityDetail: (provider: string, activityId: string) =>
      ['home', 'activity-detail', provider, activityId] as const,
    /** Every activity's route: the prefix a pull to refresh or a persister filters on. */
    activityRoutes: ['home', 'activity-route'] as const,
    /**
     * The active plan for today. Keyed by locale because the server names the
     * plan's flavour in it — a language switch is a different card.
     */
    trainingPlan: (locale: string) => ['home', 'training-plan', locale] as const,
    /** Form today, its trend and the load against baseline. Numbers and band names only, so no locale. */
    trainingStatus: () => ['home', 'training-status'] as const,
    /** The weekly volume. Numbers and sport keys only, so no locale. */
    trainingVolume: () => ['home', 'training-volume'] as const,
    /** What the athlete chose about Home: whether the plan suggestion and Getting started are set aside. */
    preferences: () => ['home', 'preferences'] as const,
    /**
     * Getting started's five rows and where the athlete stands on them —
     * one key, so Today's section and the narrow-screen Today line agree.
     */
    gettingStarted: () => ['home', 'getting-started'] as const,
    /**
     * The pending coach's « Préparer ton coaching » checklist (carnet#1529):
     * the seat, the athlete invited, a coaching platform linked.
     */
    coachSetup: () => ['home', 'coach-setup'] as const,
    /**
     * The workouts and plan weeks over the days `from..=to` (`YYYY-MM-DD`) — one strip week or one month grid —
     * with a plan day's evidence chip and relabel line in `locale`.
     */
    calendar: (from: string, to: string, locale: string) => ['home', 'calendar', from, to, locale] as const,
    /** Every calendar span: the prefix a pull to refresh or a return to Home invalidates. */
    calendars: ['home', 'calendar'] as const,
  },

  // ==================== GEAR LEDGER ====================
  gear: {
    all: ['gear'] as const,
    /** Every pair the athlete declared, retired ones included, with its distance. */
    list: () => ['gear', 'list'] as const,
    /** The pair one activity was run in. Keyed by provider as well as id, like the activity's view. */
    activity: (provider: string, activityId: string) => ['gear', 'activity', provider, activityId] as const,
  },

  // ==================== IMPORTS ====================
  imports: {
    all: ['imports'] as const,
    /** Whether imports run here, and each source's running or latest import. */
    overview: () => ['imports', 'overview'] as const,
    /** One source's running or latest import, polled while it runs. */
    current: (source: string) => ['imports', 'current', source] as const,
    /** A page of the rows one import did not import. */
    report: (source: string, importId: string) => ['imports', 'report', source, importId] as const,
  },

  // ==================== CHAT ====================
  chat: {
    all: ['chat'] as const,
    conversations: () => ['chat-conversations'] as const,
    /** The caller's one personal thread, as the server resolves it. */
    personal: () => ['chat-personal'] as const,
    messages: (conversationId: string | null) => ['chat-messages', conversationId] as const,
    /**
     * The older windows of a thread the reader scrolled back through, joined
     * before the newest window `messages` holds. Kept apart so a re-read of
     * the newest window never drops what was read back.
     */
    earlierMessages: (conversationId: string | null) =>
      ['chat-messages-earlier', conversationId] as const,
    /** A thread's topics — one per divider, plus its opening — newest first. */
    topics: (conversationId: string | null) => ['chat-topics', conversationId] as const,
    /** Who can read and post in the conversation, owner first. */
    participants: (conversationId: string | null) =>
      ['chat-participants', conversationId] as const,
    /**
     * The caller's runnable slash commands. Keyed by conversation because the
     * server narrows group-scoped commands to the group that conversation is
     * bound to — the same athlete gets a different list in a group thread.
     */
    commands: (conversationId?: string | null) => ['chat-commands', conversationId ?? null] as const,
    /** The claim-verification verdict rows of every reply in the conversation. */
    verdicts: (conversationId: string | null) => ['chat', 'verdicts', conversationId] as const,
    /** The use-case starters an empty thread offers, ranked from the athlete's state. */
    starters: () => ['chat', 'starters'] as const,
    /** A coach's plan thread: its athlete's live plan and the coach's draft over it. */
    subjectPlan: (conversationId: string | null) => ['chat', 'subject-plan', conversationId] as const,
    /** The workout families a coach's plan thread offers its editor, in the coach's language (carnet#1515). */
    subjectFamilies: (conversationId: string | null, locale: string) =>
      ['chat', 'subject-families', conversationId, locale] as const,
    /** One build's preview for one day of a coach's plan thread, in the coach's language (carnet#1515). */
    sessionPreview: (conversationId: string | null, date: string, build: unknown, locale: string) =>
      ['chat', 'session-preview', conversationId, date, build, locale] as const,
    /** A coached athlete's live plan, read-only, as their coach reads it from Group info. */
    athletePlan: (athleteUserId: string | null, groupId: string | null) =>
      ['chat', 'athlete-plan', athleteUserId, groupId] as const,
    /**
     * A coached athlete's thresholds as their coach reads them from Group
     * info (carnet#1561), worded in the coach's language and units, both in
     * the key as the athlete's own read keys them.
     */
    athletePhysiology: (athleteUserId: string | null, groupId: string | null, locale: string, units: string) =>
      ['chat', 'athlete-physiology', athleteUserId, groupId, locale, units] as const,
    /**
     * Who a coach's next send in a one-athlete room reaches — everyone, or the
     * agent alone — as this device remembers it for the viewer in that room
     * (carnet#1249). The private rows themselves are the work thread's
     * `messages`.
     */
    privateLaneAudience: (viewerId: string | null, groupId: string | null) =>
      ['chat', 'private-lane-audience', viewerId, groupId] as const,
  },

  // ==================== COACHES ====================
  coaches: {
    all: ['coaches'] as const,
    lists: () => ['coaches', 'user-coaches'] as const,
    list: (category?: string, favoritesOnly?: boolean, personalize?: boolean) =>
      ['coaches', 'user-coaches', category, favoritesOnly, personalize] as const,
    assignments: (agentId?: string) => ['coaches', 'coach-assignments', agentId] as const,
    /** The team onboarding proposes: a one-time snapshot of the athlete's data, never invalidated. */
    proposal: () => ['coaches', 'proposal'] as const,
  },

  // ==================== PERSONAS ====================
  personas: {
    all: ['personas'] as const,
    /**
     * The persona cards for one locale. Keyed by locale because the server
     * renders every word on them — a language switch is a different list, not
     * a re-translation of the one already cached.
     */
    list: (locale: string) => ['personas', locale] as const,
  },

  // ==================== COACH STORE ====================
  store: {
    all: ['store'] as const,
    coaches: (category?: string, sort?: string) =>
      ['store-coaches', category, sort] as const,
    search: (query: string) => ['store-search', query] as const,
    coach: (agentId: string) => ['store-coach', agentId] as const,
    coachDetail: (agentId?: string) => ['store-coach', agentId] as const,
    installations: () => ['store-installations'] as const,
  },

  // ==================== ADMIN - STORE ====================
  adminStore: {
    all: ['admin-store'] as const,
    stats: () => ['admin-store-stats'] as const,
    reviewQueue: () => ['admin-store-review-queue'] as const,
    published: (sortBy?: string) => ['admin-store-published', sortBy] as const,
    rejected: () => ['admin-store-rejected'] as const,
  },

  // ==================== ADMIN - COACHES ====================
  adminCoaches: {
    all: ['admin-coaches'] as const,
    system: () => ['admin-system-coaches'] as const,
    allUsers: () => ['admin-all-users'] as const,
  },

  // ==================== ADMIN - USERS ====================
  adminUsers: {
    all: ['admin-users'] as const,
    list: () => ['all-users'] as const,
    pending: () => ['pending-users'] as const,
    preApproved: () => ['pre-approved-emails'] as const,
    coachAccess: (status?: string) => ['coach-access-requests', status] as const,
    coachSeatInvitations: () => ['coach-seat-invitations'] as const,
    rateLimit: (userId?: string) => ['user-rate-limit', userId] as const,
    activity: (userId?: string) => ['user-activity', userId] as const,
    /** One user's usage figures, in the admin's user drawer. */
    usage: (userId?: string) => ['adminUsers', 'usage', userId] as const,
    /** One user's admin-only profile (persona, memories), in the same drawer. */
    adminProfile: (userId?: string) => ['adminUsers', 'admin-profile', userId] as const,
  },

  // ==================== ADMIN - SCOPED SCREENS ====================
  // Each read names the tenant scope it was made in (`tenantId` omitted is
  // every tenant), so picking another scope is another cache entry.
  adminScoped: {
    /** The tenants the scope picker offers. */
    scopeTenants: () => ['admin', 'scope-tenants'] as const,
    engagement: (tenantId?: string) => ['admin', 'engagement', tenantId] as const,
    toolUsage: (days: number, tenantId?: string) => ['admin', 'tool-usage', days, tenantId] as const,
    memoryWorkerMetrics: (tenantId?: string) => ['admin', 'memory-worker-metrics', tenantId] as const,
    verdictStats: (tenantId?: string) => ['admin', 'evals', 'verdict-stats', tenantId] as const,
  },

  // ==================== ADMIN - CLAIM VERDICTS ====================
  // The three `all*` keys are the prefixes a disposition change invalidates:
  // every page, every health window and every message lookup, whatever its
  // scope and filters.
  adminClaimVerdicts: {
    allPages: () => ['admin', 'claim-verdicts'] as const,
    /** One page of verdicts: the tenant scope, then the filters in the order the table shows them. */
    page: (
      tenantId: string | undefined,
      filters: {
        status: string;
        category: string;
        coach: string;
        layer: string;
        disposition: string;
        limit: number;
      },
    ) =>
      [
        'admin',
        'claim-verdicts',
        tenantId,
        filters.status,
        filters.category,
        filters.coach,
        filters.layer,
        filters.disposition,
        filters.limit,
      ] as const,
    allHealth: () => ['admin', 'claim-verdicts-health'] as const,
    health: (tenantId: string | undefined, windowDays: number) =>
      ['admin', 'claim-verdicts-health', tenantId, windowDays] as const,
    allMessages: () => ['admin', 'claim-verdicts-message'] as const,
    /** Every verdict on one message, looked up by its id. */
    message: (tenantId: string | undefined, messageId: string) =>
      ['admin', 'claim-verdicts-message', tenantId, messageId] as const,
    /** One verdict's detail, in the tenant that holds it. */
    detail: (tenantId: string | undefined, verdictId: string) =>
      ['admin', 'claim-verdict', tenantId, verdictId] as const,
  },

  // ==================== ADMIN - TOKENS ====================
  adminTokens: {
    all: ['admin-tokens'] as const,
    list: (includeInactive?: boolean) => ['admin-tokens', includeInactive] as const,
  },

  // ==================== ADMIN - SETTINGS ====================
  adminSettings: {
    all: ['admin-settings'] as const,
    autoApproval: () => ['auto-approval-setting'] as const,
    /** The sign-up guards: kill switch, daily ceiling, activate-on-verified-email. */
    signup: () => ['signup-setting'] as const,
  },

  // ==================== ADMIN - TOOLS ====================
  adminTools: {
    all: ['admin-tools'] as const,
    globalDisabled: () => ['global-disabled-tools'] as const,
    tenant: (tenantId: string) => ['tenant-tools', tenantId] as const,
    summary: (tenantId: string) => ['tool-availability-summary', tenantId] as const,
  },

  // ==================== DASHBOARD ====================
  dashboard: {
    all: ['dashboard'] as const,
    usageAnalytics: (days?: number) => ['usage-analytics', days] as const,
    recentActivity: (tenantId?: string) => ['admin-recent-activity', tenantId] as const,
  },

  // ==================== A2A (Agent-to-Agent) ====================
  a2a: {
    all: ['a2a'] as const,
    clients: () => ['a2a-clients'] as const,
    clientUsage: (clientId?: string) => ['a2a-client-usage', clientId] as const,
    clientRateLimit: (clientId?: string) => ['a2a-client-rate-limit', clientId] as const,
  },

  // ==================== MCP TOKENS ====================
  mcpTokens: {
    all: ['mcp-tokens'] as const,
    list: () => ['mcp-tokens'] as const,
  },

  // ==================== NOTIFICATIONS ====================
  notifications: {
    all: ['notifications'] as const,
    feed: (category?: string) => ['notifications-feed', category] as const,
    unreadCount: () => ['notifications-unread-count'] as const,
    preferences: () => ['notifications-preferences'] as const,
    devices: () => ['notifications-devices'] as const,
  },

  // ==================== GROUPS ====================
  /**
   * The group invite card and the held invite (carnet#1649, #1650). Kept
   * apart from `groups`: every group invalidation refetches that prefix, and
   * a refetched preview counts as another "opened" for the coach.
   */
  groupInvite: {
    /** The public card a link opens on: read once per page load. */
    preview: (code: string) => ['group-invite', 'preview', code] as const,
    /** The invite the server holds for the signed-in account. */
    held: () => ['group-invite', 'held'] as const,
  },
  groups: {
    all: ['groups'] as const,
    detail: (groupId: string) => ['groups', groupId] as const,
    members: (groupId: string) => ['groups', groupId, 'members'] as const,
    stats: (groupId: string) => ['groups', groupId, 'stats'] as const,
    report: (groupId: string) => ['groups', groupId, 'report'] as const,
    health: (groupId: string) => ['groups', groupId, 'health'] as const,
    invites: (groupId: string) => ['groups', groupId, 'invites'] as const,
    transcript: (groupId: string) => ['groups', groupId, 'transcript'] as const,
    /** The room a group thread renders, paged back to the thread's own history. */
    room: (groupId: string) => ['groups', groupId, 'room'] as const,
    delegatedConnections: (groupId: string) =>
      ['groups', groupId, 'delegated-connections'] as const,
    delegationRoster: (groupId: string) => ['groups', groupId, 'delegation-roster'] as const,
    coachConsents: (groupId: string) => ['groups', groupId, 'coach-consents'] as const,
    coachSharing: (groupId: string) => ['groups', groupId, 'coach-sharing'] as const,
    coachReads: (groupId: string) => ['groups', groupId, 'coach-reads'] as const,
    /** The group chat in a chat app the group's room is connected to (carnet#1683). */
    chatConnection: (groupId: string) => ['groups', groupId, 'chat-connection'] as const,
    permissions: () => ['groups', 'permissions'] as const,
  },

  // ==================== COACH REVIEW (carnet#847) ====================
  coachReplies: {
    all: ['coach-replies'] as const,
    /** The coach's pages of the agent's replies, under one athlete filter. */
    list: (athleteId: string | null) => ['coach-replies', 'list', athleteId ?? 'all'] as const,
    /** The coach's flags and corrections per week. */
    weekly: () => ['coach-replies', 'weekly'] as const,
  },

  // ==================== COACH HOME (carnet#1250, #1252, #1253) ====================
  coachRoster: {
    all: ['coach-roster'] as const,
    /** The coach's whole roster, every page read and merged. */
    list: () => ['coach-roster', 'list'] as const,
  },

  // ==================== MESSAGING ====================
  messaging: {
    all: ['messaging'] as const,
    channels: () => ['messaging-channels'] as const,
    /** The chat apps this account has linked. */
    links: () => ['messaging-links'] as const,
    /** The chat apps this deployment offers to link. */
    availableChannels: () => ['messaging-available-channels'] as const,
    /** One channel's link start: the code, deep link or QR the athlete follows. */
    linkInit: (channel: string) => ['messaging-link-init', channel] as const,
  },

  // ==================== MEMORY ====================
  memory: {
    /** Every memory-fact read; invalidating it rereads the list and the drafts. */
    facts: () => ['memory', 'facts'] as const,
    /** The saved facts, narrowed to one kind (`''` for every kind). */
    factList: (kind: string) => ['memory', 'facts', kind] as const,
    /** The facts waiting for review; under `facts()`, so keeping or discarding one rereads both lists. */
    drafts: () => ['memory', 'facts', 'drafts'] as const,
  },

  // ==================== PAR-Q ====================
  parq: {
    /** The health-screening questionnaire, in the reader's language. */
    questions: () => ['parq-questions'] as const,
  },

  // ==================== BILLING ====================
  billing: {
    all: ['billing'] as const,
    subscription: () => ['billing', 'subscription'] as const,
    invoices: () => ['billing', 'invoices'] as const,
    quota: () => ['billing', 'quota'] as const,
    plans: () => ['billing', 'plans'] as const,
  },

  // ==================== USAGE QUOTAS ====================
  usage: {
    all: ['usage'] as const,
    status: () => ['usage-status'] as const,
    llmConsumption: (days?: number) => ['llm-consumption', days] as const,
  },

  // ==================== PROMPTS ====================
  prompts: {
    all: ['prompts'] as const,
    suggestions: () => ['prompt-suggestions'] as const,
  },

  // ==================== FEATURE FLAGS ====================
  featureFlags: {
    all: ['feature-flags'] as const,
    self: () => ['feature-flags', 'me'] as const,
    tenantDefaults: (tenantId: string) => ['feature-flags', 'tenant', tenantId] as const,
    userOverrides: (userId: string) => ['feature-flags', 'user', userId] as const,
  },
} as const;

/** Type helpers for query key inference */
export type QueryKeys = typeof QUERY_KEYS;
