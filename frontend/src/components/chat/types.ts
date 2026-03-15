// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Shared types for chat components
// ABOUTME: Centralizes type definitions used across chat-related components

import type { Conversation, Coach } from '@pierre/shared-types';

export interface Message {
  id: string;
  role: 'user' | 'assistant' | 'system';
  content: string;
  token_count?: number;
  created_at: string;
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
  systemPrompt?: string;
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
};

// Re-export for convenience
export type { Conversation, Coach };
