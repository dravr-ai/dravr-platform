// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The athlete's "Who read your data" feed (GET /api/me/access-feed): each outside app's reads and agent turns, the coach's, peers' and API keys' per day
// ABOUTME: A view over the per-call records the server already keeps; nothing is written per read to build it (carnet#1543)

import type { CoachReadDataClass } from './groups.js';

/**
 * The credential an outside app read the athlete's data under. An
 * `oauth_grant` is a Connected app the athlete can revoke; every other one is
 * managed where it was made.
 */
export type AccessFeedCredential =
  | 'oauth_grant'
  | 'api_key'
  | 'mcp_token'
  | 'first_party_session'
  | 'a2a_client_credentials';

/** The door an outside app came in by: `POST /mcp`, A2A, or the athlete's API key. */
export type AccessFeedTransport = 'mcp_http' | 'a2a' | 'api_key';

/**
 * What an app read is: a tool the app called itself, or a turn of Dravr's
 * agent the app asked for (an A2A text message, a chat message through the
 * athlete's API key), which ran its own tools.
 */
export type AccessFeedReadKind = 'tool_call' | 'agent_turn';

/** One tool call, or one agent turn, an outside app made over the athlete's own data. */
export interface AccessFeedAppRead {
  /** When the call ran, or the turn ended (RFC 3339). */
  read_at: string;
  kind: AccessFeedReadKind;
  /** The tool the app called, as the server names it; null for an agent turn. */
  tool: string | null;
  /** Every tool the read ran: the one called, or the agent turn's, in call order. */
  tools: string[];
  /** The app's client id; null when the credential names no client. */
  client_id: string | null;
  /** The name the client registered for itself; null when it has none. */
  client_name: string | null;
  /** Where the client is, shown beside its self-chosen name. */
  client_host: string | null;
  /** The credential it called under; null when the server did not record one. */
  credential: AccessFeedCredential | null;
  transport: AccessFeedTransport;
  /**
   * Dravr support made the call in a support session; the client is the
   * admin's, so it is never named to the athlete.
   */
  support_read: boolean;
}

/** One day of the athlete's human coach reading their data in one group. */
export interface AccessFeedCoachDay {
  /** The UTC calendar day, `YYYY-MM-DD`. */
  day: string;
  /** The coach, by display name; null once their account is gone. */
  coach_name: string | null;
  group_id: string;
  /** The group's name; null once the group is gone. */
  group_name: string | null;
  data_class: CoachReadDataClass;
  /** Each read the coach's agent made that day. */
  reads: number;
  /** The once-a-day group views built from every member's data. */
  views: number;
  /** Some of the day's reads were served to an AI tool the coach connects to Dravr. */
  served_to_assistant: boolean;
  /** Dravr support made the day's reads acting as the coach. */
  support_read: boolean;
}

/**
 * One day of a fellow group member reading the athlete's data in one group,
 * under the athlete's peer sharing.
 */
export interface AccessFeedPeerDay {
  /** The UTC calendar day, `YYYY-MM-DD`. */
  day: string;
  /** The member who read, by display name; null once their account is gone. */
  member_name: string | null;
  group_id: string;
  /** The group's name; null once the group is gone. */
  group_name: string | null;
  /** Each read the member's agent made that day. */
  reads: number;
  /** The once-a-day group views (stats, report, health, the room's cards). */
  views: number;
  /** Some of the day's reads were served to an AI tool the member connects to Dravr. */
  served_to_assistant: boolean;
  /** Dravr support made the day's reads acting as the member. */
  support_read: boolean;
}

/**
 * One day of one of the athlete's API keys' REST requests that answered with
 * a result. Managed in the API-keys settings; a key's `/mcp` calls are its
 * app reads instead.
 */
export interface AccessFeedApiKeyDay {
  /** The UTC calendar day, `YYYY-MM-DD`. */
  day: string;
  key_id: string;
  /** The name the athlete gave the key. */
  key_name: string;
  /** The visible prefix the key is recognised by. */
  key_prefix: string;
  requests: number;
}

/** The athlete's feed over the last `days` days, newest first in each list. */
export interface AccessFeed {
  app_reads: AccessFeedAppRead[];
  coach_days: AccessFeedCoachDay[];
  peer_days: AccessFeedPeerDay[];
  api_key_days: AccessFeedApiKeyDay[];
  days: number;
}
