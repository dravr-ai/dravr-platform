// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Shared types for chat components
// ABOUTME: Centralizes type definitions used across chat-related components

import type { Conversation, Coach, MessageRole } from '@pierre/shared-types';

export interface Message {
  id: string;
  role: MessageRole;
  content: string;
  token_count?: number;
  created_at: string;
  /**
   * Resolved visual blocks, a JSON-encoded `RenderBlock[]`. Index-aligned with
   * the `⟦viz:N⟧` markers in `content`. Resolved server-side on every read, so
   * this is geometry rather than the spec the coach wrote.
   */
  scene_blocks?: string;
  isError?: boolean;
}

export interface ConversationListResponse {
  conversations: Conversation[];
  total: number;
}

export interface MessageMetadata {
  model: string;
  executionTimeMs: number;
}

export type MessageFeedback = 'up' | 'down' | null;

export interface OAuthNotification {
  provider: string;
  timestamp: number;
}

export interface DeleteConfirmation {
  id: string;
  title: string | null;
}

export interface CoachDeleteConfirmation {
  id: string;
  title: string;
}

export interface PendingCoachAction {
  prompt: string;
  coachId?: string;
}

export interface CoachFormData {
  title: string;
  description: string;
  system_prompt: string;
  category: string;
  startup_query: string;
  prefetch_enabled: boolean;
  activity_count: number;
  sport_types: string[];
  time_frame: string;
  detail_mode: 'summary' | 'detailed';
  athlete_profile: boolean;
  // Structured sections for marketplace-quality coaches
  purpose: string;
  when_to_use: string;
  instructions: string;
  example_inputs: string;
  example_outputs: string;
  success_criteria: string;
  /**
   * Per-turn tool-loop iteration budget for this coach, in three states.
   *
   * - `undefined` — untouched. The request omits the key, so an existing pin
   *   survives an unrelated edit and a coach without one keeps following the
   *   tenant-wide `tool_execution.max_iterations` an admin can raise.
   * - `null` — the user emptied the box. The update request sends an explicit
   *   `null`, which clears a stored pin back to inheriting; a create request
   *   has nothing to clear and omits it.
   * - a number — an explicit per-coach budget the user typed.
   */
  max_tool_iterations?: number | null;
}

export const DEFAULT_COACH_FORM_DATA: CoachFormData = {
  title: '',
  description: '',
  system_prompt: '',
  category: 'Training',
  startup_query: '',
  prefetch_enabled: false,
  activity_count: 20,
  sport_types: [],
  time_frame: '12w',
  detail_mode: 'summary',
  athlete_profile: false,
  purpose: '',
  when_to_use: '',
  instructions: '',
  example_inputs: '',
  example_outputs: '',
  success_criteria: '',
  // Left undefined so a coach created without touching the budget field
  // inherits the tenant-wide limit instead of pinning one.
  max_tool_iterations: undefined,
};

// Re-export for convenience
export type { Conversation, Coach };
