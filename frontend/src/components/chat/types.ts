// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Shared types for chat components
// ABOUTME: Centralizes type definitions used across chat-related components

import type { Conversation, Coach, MessageActions, MessageRole } from '@pierre/shared-types';

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
  /**
   * Why this row ended — `command` marks a slash-command turn; otherwise the
   * provider's own reason for an LLM row.
   */
  finish_reason?: string;
  /**
   * The controls a persisted command reply carried, so a reload draws the
   * same buttons the live turn did.
   */
  actions?: MessageActions;
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

// Re-export for convenience
export type { Conversation, Coach };
