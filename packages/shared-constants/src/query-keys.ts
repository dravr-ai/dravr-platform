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
    providerConnections: () => ['provider-connections'] as const,
    onboardingStatus: () => ['user-onboarding-status'] as const,
    coachAccessRequest: () => ['user-coach-access-request'] as const,
    /** The units the athlete reads, and the Settings choice behind them. */
    units: () => ['user-units'] as const,
    /** How often the coach checks in on its own (carnet#831). */
    checkIn: () => ['user-check-in'] as const,
    /** The athlete's feedback thread with the Dravr team (carnet#1235). */
    feedbackThread: () => ['user-feedback-thread'] as const,
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
  },

  // ==================== OAUTH ====================
  oauth: {
    all: ['oauth'] as const,
    status: () => ['oauth-status'] as const,
    connections: () => ['connections'] as const,
  },

  // ==================== PROVIDERS ====================
  providers: {
    all: ['providers'] as const,
    status: () => ['providers-status'] as const,
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
    /** The workouts and plan weeks over the days `from..=to` (`YYYY-MM-DD`) — one strip week or one month grid. */
    calendar: (from: string, to: string) => ['home', 'calendar', from, to] as const,
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
  },

  // ==================== COACHES ====================
  coaches: {
    all: ['coaches'] as const,
    lists: () => ['coaches', 'user-coaches'] as const,
    list: (category?: string, favoritesOnly?: boolean, personalize?: boolean) =>
      ['coaches', 'user-coaches', category, favoritesOnly, personalize] as const,
    assignments: (agentId?: string) => ['coaches', 'coach-assignments', agentId] as const,
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
    rateLimit: (userId?: string) => ['user-rate-limit', userId] as const,
    activity: (userId?: string) => ['user-activity', userId] as const,
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
    recentActivity: () => ['admin-recent-activity'] as const,
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

  // ==================== MESSAGING ====================
  messaging: {
    all: ['messaging'] as const,
    channels: () => ['messaging-channels'] as const,
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
