// ABOUTME: Shared TypeScript types for common API structures
// ABOUTME: Chat types, prompt suggestions, and common response patterns

import type { ChatMessageAction } from './turn.js';

// ========== CHAT TYPES ==========

/**
 * The newest row of a conversation, as a list row shows it.
 *
 * Mirrors `LastMessageResponse` in `routes/chat/dto.rs`.
 */
export interface ConversationLastMessage {
  /**
   * One line of the row's content: visual markers stripped, whitespace
   * collapsed, at most 120 characters.
   */
  preview: string;
  /** `user` or `assistant` — the only roles the list query reads. */
  role: 'user' | 'assistant';
  /** When the row was written (ISO 8601). */
  created_at: string;
}

/** A conversation in the chat system */
export interface Conversation {
  id: string;
  title: string | null;
  /** Model used for this conversation */
  model?: string;
  /** Coach ID if conversation uses a coach; resolves to the coach's
   *  system_prompt at runtime via the coaches table. */
  coach_id?: string | null;
  /** The attached coach's catalogue `@handle`, when it has one. List rows only. */
  coach_handle?: string | null;
  /** The attached coach's title, when the coach still exists. List rows only. */
  coach_title?: string | null;
  /** Coaching group ID if the conversation is group-scoped. When set,
   *  prompt assembly injects group context (members, peer training data
   *  subject to per-member consent). NULL for personal 1:1 chats. */
  group_id?: string | null;
  /** That group's name, when the group still exists. List rows only. */
  group_name?: string | null;
  /** Total tokens used in conversation */
  total_tokens?: number;
  /** Number of messages in conversation */
  message_count: number;
  /** Channel of origin: `web`/`mobile` for an in-app chat, or a messaging
   *  channel (`telegram`/`whatsapp`/…). Durable badge signal that survives a
   *  title rename; the client prefers it and falls back to the title prefix
   *  (see `resolveChannelOrigin` in `@pierre/chat-utils`). */
  channel_type?: string | null;
  /** When conversation was created */
  created_at: string;
  /** When conversation was last updated */
  updated_at: string;
  /** When last message was sent */
  last_message_at?: string | null;
  /**
   * The newest `user`/`assistant` row, shaped for the row preview; absent
   * for an empty conversation. List rows only.
   */
  last_message?: ConversationLastMessage | null;
  /**
   * `user`/`assistant` rows the caller has not read — every row when they
   * have never opened the thread. List rows only.
   */
  unread_count?: number;
}

/** What a participant may do beyond reading and posting. */
export type ConversationParticipantRole = 'owner' | 'member';

/**
 * One user who can read and post in a conversation.
 *
 * The owner is a participant like anyone else; the role only decides who may
 * delete the thread (the owner) and who may be removed from it (members).
 */
export interface ConversationParticipant {
  user_id: string;
  role: ConversationParticipantRole;
  /** The participant who added this one; the owner names themself. */
  added_by: string;
  /** When the membership was written (ISO 8601). */
  added_at: string;
}

/** The participants of one conversation, owner first. */
export interface ConversationParticipantsResponse {
  participants: ConversationParticipant[];
}

/**
 * A message role.
 *
 * `user` / `assistant` / `system` are the user-facing conversational turns.
 * `tool_call` / `tool_result` are internal LLM plumbing rows — they hold raw
 * `<tool_call>` / `<tool_result>` XML scaffolding the model uses to call tools
 * and read their output. They are persisted in the same `chat_messages` table
 * (so messaging-channel conversations replay correctly) but MUST NOT be
 * rendered in any user-facing thread. Use `filterDisplayMessages` from
 * `@pierre/chat-utils` to drop them before display.
 */
export type MessageRole =
  | 'user'
  | 'assistant'
  | 'system'
  | 'tool_call'
  | 'tool_result';

/** A message in a conversation */
export interface Message {
  id: string;
  /** Conversation this message belongs to */
  conversation_id?: string;
  role: MessageRole;
  content: string;
  token_count?: number;
  created_at: string;
  /**
   * Schema-validated structured payload (JSON string) extracted from a
   * builder-coach reply (e.g. a structured-workout plan). When present,
   * clients render it as a rich card instead of the raw text.
   */
  /** Resolved visual blocks: JSON-encoded `RenderBlock[]` from @pierre/scene-types. */
  scene_blocks?: string;
  /**
   * Why this row ended: the provider's own reason for an LLM row, or one of
   * the platform's stamps — `command` marks a slash-command turn (both the
   * `/…` line and its answer).
   */
  finish_reason?: string;
  /**
   * The controls a persisted command reply carried, so a reload draws the
   * same buttons the live turn did. Absent on a live turn, whose controls
   * ride the block list instead.
   */
  actions?: MessageActions;
  /** Model used for assistant messages */
  model?: string;
  /** Execution time in milliseconds */
  execution_time_ms?: number;
  /** Error flag for failed message responses */
  isError?: boolean;
}

/**
 * The controls persisted with a command reply.
 *
 * Mirrors `MessageActionsResponse` in `routes/chat/dto.rs`; the same shape a
 * live turn's `actions` reply block carries.
 */
export interface MessageActions {
  /** Label for the group, e.g. a picker's card title. */
  title?: string;
  /** The controls, in order. */
  actions: ChatMessageAction[];
}

/**
 * A user's thumbs up/down feedback on a single assistant message.
 *
 * Returned parallel to the messages list (not nested on {@link Message}) so
 * clients hydrate their feedback state directly from the conversation load,
 * and echoed back from the feedback upsert endpoint.
 */
export interface MessageFeedbackEntry {
  /** Message the feedback is attached to. */
  message_id: string;
  /** Rating value. */
  rating: 'up' | 'down';
  /** Optional free-text reason captured on a thumbs-down. */
  comment?: string | null;
}

// ========== PROMPT SUGGESTIONS ==========

/**
 * The canonical six fitness-adapted health pillars — the single source of truth
 * for per-user context, pillar-tagged facts, and prompt categorization. Mirrors
 * the Rust `Pillar` enum. Distinct from the coach-marketplace `CoachCategory`.
 */
export type ActivityPillar =
  | 'training_and_movement'
  | 'fuelling'
  | 'sleep_and_recovery'
  | 'mental_resilience'
  | 'community_and_connection'
  | 'recovery_optimisation';

// ========== COMMON RESPONSE TYPES ==========

/** Standard API metadata */
export interface ApiMetadata {
  timestamp: string;
  api_version: string;
}

/** Standard paginated response structure */
export interface PaginatedResponse<T> {
  items: T[];
  next_cursor: string | null;
  has_more: boolean;
  metadata: ApiMetadata;
}

/** Standard list response structure */
export interface ListResponse<T> {
  items: T[];
  total: number;
  metadata: ApiMetadata;
}

// ========== SLASH COMMANDS ==========

/**
 * One slash command the calling athlete may run.
 *
 * Every field comes out of the server's `commands/**\/*.md` frontmatter by way
 * of the registry `/help` reads, so a client that renders these is showing the
 * same catalogue messaging shows — never a copy of it.
 */
export interface CommandEntry {
  /** Catalogue id and handler-registry key (`group-invite`). */
  name: string;
  /** The string the athlete types (`/group invite`). */
  command: string;
  /**
   * Argument signature (`yes|no`, `[week|today]`), or `null` for a command
   * that takes no arguments.
   */
  args: string | null;
  /** One-line description — the same text `/help` prints. */
  description: string;
  /** Domain grouping (`general`, `group`, `coach`, `data`, …). */
  domain: string;
}

/** Response for `GET /api/commands`. */
export interface CommandCatalogueResponse {
  /**
   * The caller's runnable commands, ordered by domain then command string.
   *
   * Empty is a real answer: a server built without a command catalogue has
   * none to offer, and a client that then shows no palette is correct.
   */
  commands: CommandEntry[];
}
