// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: What web and phone share for the athlete's "Who read your data" card — its words, the credential labels, the coach's and peers' day counts
// ABOUTME: A constants module cannot translate, so keys are named here and each client resolves them (carnet#1543)

import type { CountKeys } from '@pierre/chat-utils';
import type { AccessFeedCredential } from '@pierre/shared-types';

/**
 * The card's own words, under `accessFeed.*`. None of them names the coach:
 * outside `humanCoach.*` that word means the AI persona (ADR-026), so the
 * coach's part of the card reads from {@link ACCESS_FEED_COACH_KEYS}.
 */
export const ACCESS_FEED_KEYS = {
  title: 'accessFeed.title',
  /** Interpolates `{{days}}`, the window the server answered for. */
  intro: 'accessFeed.intro',
  /** Interpolates `{{days}}`. */
  empty: 'accessFeed.empty',
  loadFailed: 'accessFeed.loadFailed',
  appsHeading: 'accessFeed.appsHeading',
  /** One call an app made; interpolates `{{tool}}`. */
  appRow: 'accessFeed.appRow',
  /** An app whose reads name neither a client name nor a client id. */
  unknownApp: 'accessFeed.unknownApp',
  /** Beside an app that came in over A2A rather than MCP. */
  overA2a: 'accessFeed.overA2a',
  /** The heading of the calls Dravr support made in a support session; the client is the admin's and is not named. */
  supportRead: 'accessFeed.supportRead',
  /** Under an app reading through an API key or an MCP token: where that access is cut off. */
  apiKeyHint: 'accessFeed.apiKeyHint',
  /** One turn of Dravr's agent an app asked for; it names no tool, since the agent chose them. */
  agentTurnRow: 'accessFeed.agentTurnRow',
  /** The heading of the fellow group members' reads, under the athlete's peer sharing. */
  peersHeading: 'accessFeed.peersHeading',
  /** Stands in `{{member}}` for a member whose account is gone. */
  formerMember: 'accessFeed.formerMember',
  /** Under a member's day some of whose reads went to an AI tool the member connects to Dravr. */
  peerServedToAssistant: 'accessFeed.peerServedToAssistant',
  /** Under a member's day whose reads Dravr support made acting as the member. */
  peerSupportRead: 'accessFeed.peerSupportRead',
} as const;

/** The coach's part of the card, under `humanCoach.readLog.*` beside the per-group record it sums. */
export const ACCESS_FEED_COACH_KEYS = {
  feedHeading: 'humanCoach.readLog.feedHeading',
  /** Stands in `{{group}}` for a group that is gone. */
  unknownGroup: 'humanCoach.readLog.unknownGroup',
} as const;

/** How many reads the coach's agent made that day in a group: « 1 lecture dans {{group}} ». */
export const ACCESS_FEED_DAY_READS_KEYS: CountKeys = {
  one: 'humanCoach.readLog.dayReads_one',
  many: 'humanCoach.readLog.dayReads_other',
};

/** How many once-a-day group views reached the athlete's data that day: « 1 consultation dans {{group}} ». */
export const ACCESS_FEED_DAY_VIEWS_KEYS: CountKeys = {
  one: 'humanCoach.readLog.dayViews_one',
  many: 'humanCoach.readLog.dayViews_other',
};

/**
 * How many times a fellow member's agent read the athlete's activities that
 * day in a group: « {{member}} a lu tes activités 3 fois dans {{group}} ».
 */
export const ACCESS_FEED_PEER_DAY_READS_KEYS: CountKeys = {
  one: 'accessFeed.peerDayReads_one',
  many: 'accessFeed.peerDayReads_other',
};

/** How many once-a-day group views of a fellow member reached the athlete's data that day: « 1 consultation dans {{group}} ». */
export const ACCESS_FEED_PEER_DAY_VIEWS_KEYS: CountKeys = {
  one: 'accessFeed.peerDayViews_one',
  many: 'accessFeed.peerDayViews_other',
};

/** How many of an API key's REST requests that day answered with a result: « 3 requêtes ». */
export const ACCESS_FEED_API_KEY_DAY_REQUESTS_KEYS: CountKeys = {
  one: 'accessFeed.apiKeyDayRequests_one',
  many: 'accessFeed.apiKeyDayRequests_other',
};

/**
 * The label of each credential an app can read under, beside its name. A
 * Connected app (`oauth_grant`) carries none: it is the ordinary case, and its
 * Revoke says what it is.
 */
const ACCESS_FEED_CREDENTIAL_KEYS: Record<Exclude<AccessFeedCredential, 'oauth_grant'>, string> = {
  api_key: 'accessFeed.credential.apiKey',
  mcp_token: 'accessFeed.credential.mcpToken',
  first_party_session: 'accessFeed.credential.firstParty',
  a2a_client_credentials: 'accessFeed.credential.a2aClient',
};

/** The key of the label a credential reads as, or `null` for a Connected app or an unrecorded one. */
export function accessFeedCredentialKey(credential: Exclude<AccessFeedCredential, 'oauth_grant'>): string;
export function accessFeedCredentialKey(credential: AccessFeedCredential | null): string | null;
export function accessFeedCredentialKey(credential: AccessFeedCredential | null): string | null {
  if (credential === null || credential === 'oauth_grant') return null;
  return ACCESS_FEED_CREDENTIAL_KEYS[credential];
}

/** Every key the card asks the catalogue for, for a test that pins them. */
export const ACCESS_FEED_ALL_KEYS: readonly string[] = [
  ...Object.values(ACCESS_FEED_KEYS),
  ...Object.values(ACCESS_FEED_COACH_KEYS),
  ACCESS_FEED_DAY_READS_KEYS.one,
  ACCESS_FEED_DAY_READS_KEYS.many,
  ACCESS_FEED_DAY_VIEWS_KEYS.one,
  ACCESS_FEED_DAY_VIEWS_KEYS.many,
  ACCESS_FEED_PEER_DAY_READS_KEYS.one,
  ACCESS_FEED_PEER_DAY_READS_KEYS.many,
  ACCESS_FEED_PEER_DAY_VIEWS_KEYS.one,
  ACCESS_FEED_PEER_DAY_VIEWS_KEYS.many,
  ACCESS_FEED_API_KEY_DAY_REQUESTS_KEYS.one,
  ACCESS_FEED_API_KEY_DAY_REQUESTS_KEYS.many,
  ...Object.values(ACCESS_FEED_CREDENTIAL_KEYS),
];
