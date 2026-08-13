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
    /** Claim verdicts attached to messages in this conversation */
    VERDICTS: (id: string) => `/api/chat/conversations/${id}/verdicts`,
    /** Set (POST) or clear (DELETE) the caller's thumbs up/down feedback on a message */
    MESSAGE_FEEDBACK: (conversationId: string, messageId: string) =>
      `/api/chat/conversations/${conversationId}/messages/${messageId}/feedback`,
  },

  // ==================== COACHES ====================
  COACHES: {
    /** List/create coaches */
    LIST: '/api/coaches',
    /** Get/update/delete a coach */
    COACH: (id: string) => `/api/coaches/${id}`,
    /** Toggle favorite status */
    FAVORITE: (id: string) => `/api/coaches/${id}/favorite`,
    /** Record coach usage */
    USAGE: (id: string) => `/api/coaches/${id}/usage`,
    /** Hide/show a coach */
    HIDE: (id: string) => `/api/coaches/${id}/hide`,
    /** List hidden coaches */
    HIDDEN: '/api/coaches/hidden',
    /** Fork a coach */
    FORK: (id: string) => `/api/coaches/${id}/fork`,
    /** List versions */
    VERSIONS: (id: string) => `/api/coaches/${id}/versions`,
    /** Get specific version */
    VERSION: (id: string, version: number) => `/api/coaches/${id}/versions/${version}`,
    /** Revert to version */
    VERSION_REVERT: (id: string, version: number) =>
      `/api/coaches/${id}/versions/${version}/revert`,
    /** Diff between versions */
    VERSION_DIFF: (id: string, fromVersion: number, toVersion: number) =>
      `/api/coaches/${id}/versions/${fromVersion}/diff/${toVersion}`,
    /** Onboarding coach proposal (inferred profile + top-3 coaches) */
    PROPOSAL: '/api/coaches/proposal',
    /** Generate coach from conversation */
    GENERATE: '/api/coaches/generate',
    /** Import a coach from markdown content */
    IMPORT: '/api/coaches/import',
    /** Preview an import before saving */
    IMPORT_PREVIEW: '/api/coaches/import/preview',
    /** Import a coach from a URL */
    IMPORT_URL: '/api/coaches/import/url',
    /** Export a coach as markdown */
    EXPORT: (id: string) => `/api/coaches/${id}/export`,
  },

  // ==================== PROMPTS ====================
  PROMPTS: {
    /** Get prompt suggestions */
    SUGGESTIONS: '/api/social/insights/suggestions',
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

  // ==================== SOCIAL ====================
  SOCIAL: {
    /** List friends */
    FRIENDS: '/api/social/friends',
    /** Pending friend requests (received) */
    FRIENDS_PENDING: '/api/social/friends/pending',
    /** Friend requests (sent/received) */
    FRIENDS_REQUESTS: '/api/social/friends',
    /** Specific friend request */
    FRIEND_REQUEST: (id: string) => `/api/social/friends/${id}`,
    /** Accept friend request */
    FRIEND_REQUEST_ACCEPT: (id: string) => `/api/social/friends/${id}/accept`,
    /** Reject/decline friend request */
    FRIEND_REQUEST_REJECT: (id: string) => `/api/social/friends/${id}/decline`,
    /** Specific friend (for removal) */
    FRIEND: (id: string) => `/api/social/friends/${id}`,
    /** Block a user */
    FRIEND_BLOCK: (id: string) => `/api/social/friends/${id}/block`,
    /** Search users */
    USER_SEARCH: '/api/social/users/search',
    /** Social feed */
    FEED: '/api/social/feed',
    /** Share an insight */
    SHARE: '/api/social/insights',
    /** Specific shared insight */
    SHARED_INSIGHT: (id: string) => `/api/social/insights/${id}`,
    /** List my insights */
    INSIGHTS: '/api/social/insights',
    /** Specific insight */
    INSIGHT: (id: string) => `/api/social/insights/${id}`,
    /** Reactions on an insight */
    INSIGHT_REACTIONS: (id: string) => `/api/social/insights/${id}/reactions`,
    /** Adapt an insight */
    INSIGHT_ADAPT: (id: string) => `/api/social/insights/${id}/adapt`,
    /** Get adapted insight */
    ADAPT: (id: string) => `/api/social/insights/${id}/adapt`,
    /** List adapted insights */
    ADAPTED: '/api/social/adapted',
    /** Social settings */
    SETTINGS: '/api/social/settings',
    /** Insight suggestions (coach-generated) */
    SUGGESTIONS: '/api/social/insights/suggestions',
    /** Share insight from activity */
    FROM_ACTIVITY: '/api/social/insights/from-activity',
    /** Generate shareable insight from analysis content */
    GENERATE: '/api/social/insights/generate',
  },

  // ==================== STORE ====================
  STORE: {
    /** Browse/list store coaches */
    COACHES: '/api/store/coaches',
    /** Get specific store coach */
    COACH: (id: string) => `/api/store/coaches/${id}`,
    /** Search store coaches */
    SEARCH: '/api/store/search',
    /** Get store categories */
    CATEGORIES: '/api/store/categories',
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
    /** List/create groups */
    LIST: '/api/groups',
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
    /** List/create invites */
    INVITES: (id: string) => `/api/groups/${id}/invites`,
    /** Specific invite */
    INVITE: (groupId: string, inviteId: string) => `/api/groups/${groupId}/invites/${inviteId}`,
    /** Join a group via invite code */
    JOIN: '/api/groups/join',
    /** Detach the human coach (admin/owner only) */
    COACH: (id: string) => `/api/groups/${id}/coach`,
    /** List groups the current user is the human coach of */
    COACHED: '/api/groups/coached',
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
