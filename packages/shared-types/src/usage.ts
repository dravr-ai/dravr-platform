// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The quota counters GET /api/usage/status serves — the chat banner and both settings usage cards read them
// ABOUTME: Mirrors pierre_server::routes::usage::UsageStatusResponse and pierre_services::usage_counter::LimitCheckResult

/** One counter's limit check. */
export interface LimitCheckResult {
  /** Whether the request is allowed (under the hard limit). */
  allowed: boolean;
  /** Current counter value. */
  current: number;
  /** Configured soft limit. */
  limit: number;
  /** Whether the user is approaching the limit (at or above the warning threshold). */
  warning: boolean;
  /** Whether the user is in the burst zone (between the limit and the hard limit). */
  burst_zone: boolean;
  /** ISO 8601 instant the counter resets at. */
  resets_at: string;
}

/** Every counter and resource cap for the calling user. */
export interface UsageStatusResponse {
  daily: {
    messages: LimitCheckResult;
    tokens: LimitCheckResult;
    tool_calls: LimitCheckResult;
  };
  weekly: {
    messages: LimitCheckResult;
    tokens: LimitCheckResult;
    tool_calls: LimitCheckResult;
  };
  resources: {
    conversations: number;
    max_conversations: number;
    agents: number;
    max_agents: number;
  };
}
