// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: API endpoint URL constants shared between platforms
// ABOUTME: Centralizes all endpoint definitions to avoid duplication

/**
 * API endpoint constants.
 * All endpoints are relative to the base URL configured in the platform adapter.
 */
export const ENDPOINTS = {
  // ==================== AUTH ====================
  AUTH: {
    /** OAuth token endpoint (login) */
    TOKEN: '/oauth/token',
    /** Firebase authentication */
    FIREBASE: '/api/auth/firebase',
    /** Logout */
    LOGOUT: '/api/auth/logout',
    /** User registration */
    REGISTER: '/api/auth/register',
    /** Token refresh */
    REFRESH: '/api/auth/refresh',
    /** Restore session from httpOnly cookie */
    SESSION: '/api/auth/session',
    /** Self-service forgot password (send reset code) */
    FORGOT_PASSWORD: '/api/auth/forgot-password',
    /** Complete password reset with code */
    COMPLETE_RESET: '/api/auth/complete-reset',
    /** Re-send the post-registration address-confirmation link */
    RESEND_VERIFICATION: '/api/auth/resend-verification',
  },

  // ==================== CHAT ====================
  CHAT: {
    /** List/create conversations */
    CONVERSATIONS: '/api/chat/conversations',
    /** Get/update/delete a conversation */
    CONVERSATION: (id: string) => `/api/chat/conversations/${id}`,
    /** Get/send messages in a conversation */
    MESSAGES: (id: string) => `/api/chat/conversations/${id}/messages`,
    /** List (GET) / add (POST) the participants of a conversation */
    PARTICIPANTS: (id: string) => `/api/chat/conversations/${id}/participants`,
    /** Remove (DELETE) one participant from a conversation */
    PARTICIPANT: (conversationId: string, userId: string) =>
      `/api/chat/conversations/${conversationId}/participants/${userId}`,
    /** Claim verdicts attached to messages in this conversation */
    VERDICTS: (id: string) => `/api/chat/conversations/${id}/verdicts`,
    /** Set (POST) or clear (DELETE) the caller's thumbs up/down feedback on a message */
    MESSAGE_FEEDBACK: (conversationId: string, messageId: string) =>
      `/api/chat/conversations/${conversationId}/messages/${messageId}/feedback`,
    /** Advance (POST) or clear (DELETE) the caller's read marker on a conversation */
    READ: (id: string) => `/api/chat/conversations/${id}/read`,
  },

  // ==================== SLASH COMMANDS ====================
  /**
   * The slash commands the calling athlete may actually run.
   *
   * Resolved per caller by the same availability predicates `/help` asks each
   * handler, so a palette built from it never offers a command the caller
   * would be refused. Not under CHAT because a command is not a turn — the
   * palette opens before there is anything to send, and outside a conversation
   * there is no conversation id to hang it from.
   */
  COMMANDS: '/api/commands',

  // ==================== COACHES ====================
  COACHES: {
    /** List the caller's coaches */
    LIST: '/api/coaches',
    /** Get/update/delete a coach */
    COACH: (id: string) => `/api/coaches/${id}`,
    /** Record coach usage */
    USAGE: (id: string) => `/api/coaches/${id}/usage`,
    /** Onboarding coach proposal (inferred profile + top-3 coaches) */
    PROPOSAL: '/api/coaches/proposal',
  },

  // ==================== OAUTH ====================
  OAUTH: {
    /** Get OAuth connection status */
    STATUS: '/api/oauth/status',
    /** Initialize mobile OAuth flow */
    MOBILE_INIT: (provider: string) => `/api/oauth/mobile/init/${provider}`,
    /** Disconnect provider (revoke tokens) */
    DISCONNECT: (provider: string) => `/api/oauth/providers/${provider}/disconnect`,
    /** List the caller's connected MCP OAuth apps (approved on the consent screen) */
    CONNECTED_APPS: '/api/me/oauth-grants',
    /** Revoke one connected MCP OAuth app grant by id */
    CONNECTED_APP: (id: string) => `/api/me/oauth-grants/${id}`,
  },

  // ==================== PROVIDERS ====================
  PROVIDERS: {
    /** Get all providers with connection status (OAuth and non-OAuth) */
    STATUS: '/api/providers',
    /** Link an Intervals.icu account with athlete id + API key (non-OAuth) */
    INTERVALS_ICU_LINK: '/api/providers/intervals_icu/link-credentials',
    /** Disconnect the linked Intervals.icu account */
    INTERVALS_ICU_DISCONNECT: '/api/providers/intervals_icu/disconnect',
  },

  // ==================== STORE ====================
  STORE: {
    /** Browse/list store coaches */
    COACHES: '/api/store/coaches',
    /** Get specific store coach */
    COACH: (id: string) => `/api/store/coaches/${id}`,
    /** Search store coaches */
    SEARCH: '/api/store/search',
    /** Install/uninstall a coach */
    INSTALL: (id: string) => `/api/store/coaches/${id}/install`,
    /** List installed coaches */
    INSTALLATIONS: '/api/store/installations',
  },

  // ==================== NOTIFICATIONS ====================
  NOTIFICATIONS: {
    /** List notifications (feed) */
    FEED: '/api/notifications',
    /** Get unread notification count */
    UNREAD_COUNT: '/api/notifications/unread-count',
    /** Mark all notifications as read */
    READ_ALL: '/api/notifications/read-all',
    /** Mark a single notification as read */
    READ: (id: string) => `/api/notifications/${id}/read`,
    /** Delete a notification */
    DELETE: (id: string) => `/api/notifications/${id}`,
    /** Register/list device tokens */
    DEVICES: '/api/notifications/device',
    /** Deactivate a specific device token */
    DEVICE: (id: string) => `/api/notifications/device/${id}`,
    /** Get/update notification preferences */
    PREFERENCES: '/api/notifications/preferences',
    /** Sync badge count with server */
    BADGE_SYNC: '/api/notifications/badge-sync',
  },

  // ==================== GROUPS ====================
  GROUPS: {
    /** Get/update/delete a group */
    GROUP: (id: string) => `/api/groups/${id}`,
    /** List/manage members */
    MEMBERS: (id: string) => `/api/groups/${id}/members`,
    /** Specific member */
    MEMBER: (groupId: string, userId: string) => `/api/groups/${groupId}/members/${userId}`,
    /** Update member role */
    MEMBER_ROLE: (groupId: string, userId: string) =>
      `/api/groups/${groupId}/members/${userId}/role`,
    /** Update own peer sharing consent */
    MY_CONSENT: (id: string) => `/api/groups/${id}/members/me/consent`,
    /** The shared room transcript (membership-gated, consent-filtered) */
    TRANSCRIPT: (id: string) => `/api/chat/groups/${id}/transcript`,
    /** List/create invites */
    INVITES: (id: string) => `/api/groups/${id}/invites`,
    /** Specific invite */
    INVITE: (groupId: string, inviteId: string) => `/api/groups/${groupId}/invites/${inviteId}`,
    /** Detach the human coach (admin/owner only) */
    COACH: (id: string) => `/api/groups/${id}/coach`,
    /** Check group creation permissions */
    PERMISSIONS: '/api/groups/permissions',
    /** Leave a group */
    LEAVE: (id: string) => `/api/groups/${id}/leave`,
    /** Group aggregate stats */
    STATS: (id: string) => `/api/groups/${id}/stats`,
    /** Group weekly report */
    REPORT: (id: string) => `/api/groups/${id}/report`,
    /** Group health flags */
    HEALTH: (id: string) => `/api/groups/${id}/health`,
  },

  // ==================== USER ====================
  USER: {
    /** User profile */
    PROFILE: '/api/user/profile',
    /** User stats */
    STATS: '/api/user/stats',
    /** MCP tokens */
    MCP_TOKENS: '/api/user/mcp-tokens',
    /** Specific MCP token */
    MCP_TOKEN: (id: string) => `/api/user/mcp-tokens/${id}`,
    /** Change password */
    CHANGE_PASSWORD: '/api/user/change-password',
    /** Analytics consent */
    ANALYTICS_CONSENT: '/api/user/analytics-consent',
    /**
     * Reply language (`users.locale`) — the language the coach answers in.
     * Clients PUT this whenever the viewer changes the app language, so the
     * chrome and the coach never speak two different languages.
     */
    LOCALE: '/api/user/locale',
    /**
     * Theme preference (`users.theme`) — `light`/`dark` pins a scheme across
     * devices, `null` clears the pin back to following the system. Clients
     * PUT this whenever the viewer changes the theme control.
     */
    THEME: '/api/user/theme',
    /** Coaching persona (output format / cadence) */
    COACHING_PERSONA: '/api/user/coaching-persona',
    /** LLM settings */
    LLM_SETTINGS: '/api/user/llm-settings',
    /** Validate LLM settings */
    LLM_SETTINGS_VALIDATE: '/api/user/llm-settings/validate',
    /** Provider-specific LLM settings */
    LLM_SETTINGS_PROVIDER: (provider: string) => `/api/user/llm-settings/${provider}`,
    /** User OAuth apps */
    OAUTH_APPS: '/api/users/oauth-apps',
    /** Specific OAuth app */
    OAUTH_APP: (provider: string) => `/api/users/oauth-apps/${provider}`,
    /** PAR-Q+ pre-participation questions (GET) and answers (POST) */
    PARQ: '/api/me/parq',
    /** About-you onboarding answers — North Star, primary sport, goal */
    ABOUT_YOU: '/api/me/about-you',
    /** Onboarding status — cheap self-read used by web + mobile to gate routing right after login */
    ONBOARDING_STATUS: '/api/me/onboarding-status',
    /** Durable per-step onboarding progress — clients PUT a step's status as it completes */
    ONBOARDING_STEP: (stepId: string) => `/api/me/onboarding/steps/${encodeURIComponent(stepId)}`,
    /** IANA timezone setter — clients PUT this right after login so the chat prompt can render {{CURRENT_DATE}} in the user's local calendar */
    TIMEZONE: '/api/users/me/timezone',
  },
  // ==================== FEATURE FLAGS ====================
  FEATURE_FLAGS: {
    /** Effective feature flags for the calling user, plus the known-flag registry */
    ME: '/api/me/features',
  },

  /** End-user messaging channel linking (onboarding) — not the admin channel-config surface */
  MESSAGING: {
    /** Secret-free connectable-channel list for the onboarding picker */
    CHANNELS_AVAILABLE: '/api/messaging/channels/available',
    /** Start linking a channel — returns linking URL/code (+ QR for deep-link) */
    LINK_INIT: (channel: string) => `/api/messaging/link/init/${encodeURIComponent(channel)}`,
    /** The channels the user has already linked (poll to detect link completion) */
    LINKS: '/api/messaging/links',
    /** Unlink a channel */
    LINK: (channel: string) => `/api/messaging/links/${encodeURIComponent(channel)}`,
  },
} as const;

/** Type for endpoint keys */
export type EndpointKeys = typeof ENDPOINTS;
