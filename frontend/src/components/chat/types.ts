// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Shared types for chat components
// ABOUTME: Centralizes type definitions used across chat-related components

import type { Conversation, Agent, Message, MessageOrigin } from '@pierre/shared-types';

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

/**
 * A composer action a host hands the chat thread.
 *
 * `draft` seeds the composer and leaves the athlete to press send; `send`
 * dispatches the text as a turn on its own. Both start a conversation first
 * when none is open — an invite link opened cold has no thread to speak into.
 */
export interface PendingComposerAction {
  kind: 'draft' | 'send';
  text: string;
  /**
   * How a `send` was produced, when not typed: the activity view's chips say
   * `chip`. A `draft` is reported as one when the athlete sends it.
   */
  origin?: MessageOrigin;
  /**
   * What the athlete's bubble shows for a `send` whose text is a starter's
   * postback: its label, until the turn echoes the words it stands for.
   */
  display?: string;
}

// Re-export for convenience
export type { Conversation, Agent, Message };
