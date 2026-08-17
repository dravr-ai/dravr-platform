// ABOUTME: Shared TypeScript types for common API structures
// ABOUTME: Chat types, prompt suggestions, and common response patterns

// ========== CHAT TYPES ==========

/** A conversation in the chat system */
export interface Conversation {
  id: string;
  title: string | null;
  /** Model used for this conversation */
  model?: string;
  /** Coach ID if conversation uses a coach; resolves to the coach's
   *  system_prompt at runtime via the coaches table. */
  coach_id?: string | null;
  /** Coaching group ID if the conversation is group-scoped. When set,
   *  prompt assembly injects group context (members, peer training data
   *  subject to per-member consent). NULL for personal 1:1 chats. */
  group_id?: string | null;
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
  /** Model used for assistant messages */
  model?: string;
  /** Execution time in milliseconds */
  execution_time_ms?: number;
  /** Error flag for failed message responses */
  isError?: boolean;
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

/** A category of prompt suggestions */
export interface PromptCategory {
  category_key: string;
  category_title: string;
  category_icon: string;
  pillar: ActivityPillar;
  prompts: string[];
}

/** Response for prompt suggestions */
export interface PromptSuggestionsResponse {
  categories: PromptCategory[];
  welcome_prompt: string;
  metadata: {
    timestamp: string;
    api_version: string;
  };
}

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
