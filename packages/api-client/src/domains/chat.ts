// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Chat domain API - conversations and messages
// ABOUTME: Handles CRUD for chat conversations and sending/receiving messages

import type { AxiosInstance } from 'axios';
import type { Conversation, Message } from '@pierre/shared-types';
import { ENDPOINTS } from '../core/endpoints';

// Re-export types for consumers
export type { Conversation, Message };

export interface ConversationsResponse {
  conversations: Conversation[];
  total: number;
  limit: number;
  offset: number;
}

export interface MessagesResponse {
  messages: Message[];
}

/**
 * User-facing wire shape for a Tier 5.5 claim verdict attached to a
 * conversation message. Mirrors the admin row shape but stays inside the
 * chat domain so the dispatch UI can render chips without going through
 * the admin permission gate.
 */
export interface ChatVerdictRow {
  id: string;
  conversation_id: string | null;
  message_id: string | null;
  coach_id: string | null;
  claim_text: string;
  category:
    | 'physiological'
    | 'training_prescription'
    | 'nutrition'
    | 'recovery'
    | 'supplement'
    | 'injury_rehab';
  status: 'supported' | 'unsupported' | 'contradicted' | 'rhetorical' | 'unverifiable';
  evidence_strength: 'strong' | 'mixed' | 'weak' | 'none';
  confidence: number;
  layer_fired: 'rhetoric' | 'deterministic' | 'evidence' | 'consistency' | 'judge';
  explanation: string | null;
  evidence_refs: string | null;
  created_at: string;
}

export interface ChatVerdictsResponse {
  verdicts: ChatVerdictRow[];
  total: number;
}

/**
 * Interactive button attached to a command-response turn.
 *
 * Returned by the server when a slash command like `/coach` produces a
 * card shape with selectable options. Frontends render each action as
 * a clickable button; clicking it re-POSTs `value` as the user's next
 * message, flowing back through the same dispatcher.
 */
export interface ChatMessageAction {
  /** User-visible button label. */
  label: string;
  /**
   * Action kind. `"postback"` means the frontend should send `value`
   * as the next chat message. `"url"` means open `value` in a browser.
   * Unknown types should be ignored.
   */
  action_type: string;
  /**
   * For `postback`: the text to POST as the next user message
   * (e.g. `/coach select <uuid>`). For `url`: the absolute URL.
   */
  value: string;
}

export interface SendMessageResponse {
  user_message: Message;
  assistant_message: Message;
  conversation_updated_at: string;
  model: string;
  execution_time_ms: number;
  /** Activity list from get_activities tool, separate from message content */
  activity_list?: string;
  /**
   * Optional card title for command responses (e.g. `/coach` returns
   * "Choose a coach"). Absent for regular LLM turns.
   */
  card_title?: string;
  /**
   * Optional action buttons for command responses. Present when the
   * command handler returned a card with selectable options (e.g. the
   * per-coach picker on `/coach`). Not persisted — only live for the
   * turn that produced them; historical messages show the rendered
   * text body without buttons.
   */
  actions?: ChatMessageAction[];
  /**
   * `true` when the assistant response came from a local slash-command
   * handler rather than the LLM. UI can skip LLM-specific treatments
   * (disclaimers, token-cost chips) on these turns.
   */
  is_command_response?: boolean;
  /**
   * Server-echoed AG-UI run id. When the client passed `agui_run_id`
   * in the request, the same UUID comes back here so the client can
   * correlate its parallel `/api/agui/runs/{run_id}/stream` subscription
   * with the response turn.
   */
  agui_run_id?: string;
}

/**
 * Optional extras accepted by {@link ChatApi.sendMessage}.
 *
 * Passing `aguiRunId` turns on AG-UI progress events: the server
 * registers the run under that UUID and emits events while the
 * pipeline executes. Callers consume them by opening an SSE stream
 * to `/api/agui/runs/{aguiRunId}/stream`. Omit the option and the
 * pipeline runs without AG-UI overhead.
 */
export interface SendMessageOptions {
  /** UUID identifying the AG-UI run for this turn. */
  aguiRunId?: string;
}

export interface CreateConversationOptions {
  title?: string;
  model?: string;
  /** Coach to attach to the conversation; the coach's system_prompt
   *  is resolved server-side at runtime. */
  coach_id?: string;
  /** Coaching group to scope the conversation to. When set, the
   *  server-side prompt-assembly stage injects group context (member
   *  roster, peer training data subject to per-member consent,
   *  role-aware summaries). The caller must be an active member of
   *  the group; the REST endpoint rejects non-members with 403. */
  group_id?: string;
}

/**
 * Creates the chat API methods bound to an axios instance.
 */
export function createChatApi(axios: AxiosInstance, getBaseUrl: () => string) {
  return {
    /**
     * List all conversations for the current user.
     */
    async getConversations(limit = 50, offset = 0): Promise<ConversationsResponse> {
      const response = await axios.get<ConversationsResponse>(
        `${ENDPOINTS.CHAT.CONVERSATIONS}?limit=${limit}&offset=${offset}`
      );
      return response.data;
    },

    /**
     * Create a new conversation.
     */
    async createConversation(options?: CreateConversationOptions): Promise<Conversation> {
      const response = await axios.post<Conversation>(
        ENDPOINTS.CHAT.CONVERSATIONS,
        options ?? {}
      );
      return response.data;
    },

    /**
     * Get a specific conversation by ID.
     */
    async getConversation(conversationId: string): Promise<Conversation> {
      const response = await axios.get<Conversation>(
        ENDPOINTS.CHAT.CONVERSATION(conversationId)
      );
      return response.data;
    },

    /**
     * Update a conversation (e.g., change title).
     */
    async updateConversation(
      conversationId: string,
      updates: { title?: string }
    ): Promise<Conversation> {
      const response = await axios.put<Conversation>(
        ENDPOINTS.CHAT.CONVERSATION(conversationId),
        updates
      );
      return response.data;
    },

    /**
     * Delete a conversation.
     */
    async deleteConversation(conversationId: string): Promise<void> {
      await axios.delete(ENDPOINTS.CHAT.CONVERSATION(conversationId));
    },

    /**
     * Get messages in a conversation.
     */
    async getConversationMessages(conversationId: string): Promise<MessagesResponse> {
      const response = await axios.get<MessagesResponse>(
        ENDPOINTS.CHAT.MESSAGES(conversationId)
      );
      return response.data;
    },

    /**
     * Fetch the Tier 5.5 claim verdicts attached to the messages in a
     * conversation. Returns an empty array if the verification pipeline
     * has not produced any verdicts yet (or the feature is disabled).
     */
    async getConversationVerdicts(conversationId: string): Promise<ChatVerdictsResponse> {
      const response = await axios.get<ChatVerdictsResponse>(
        ENDPOINTS.CHAT.VERDICTS(conversationId)
      );
      return response.data;
    },

    /**
     * Send a message in a conversation.
     *
     * Pass `options.aguiRunId` (a freshly generated UUID) to enable
     * AG-UI progress events for this turn; subscribe in parallel to
     * `/api/agui/runs/{aguiRunId}/stream` with a Bearer JWT to render
     * pipeline progress in-UI. Omit to run without progress feedback.
     */
    async sendMessage(
      conversationId: string,
      content: string,
      options?: SendMessageOptions,
    ): Promise<SendMessageResponse> {
      const body: {
        content: string;
        stream: boolean;
        agui_run_id?: string;
      } = { content, stream: false };
      if (options?.aguiRunId) {
        body.agui_run_id = options.aguiRunId;
      }
      const response = await axios.post<SendMessageResponse>(
        ENDPOINTS.CHAT.MESSAGES(conversationId),
        body,
      );
      return response.data;
    },

    /**
     * Get the WebSocket URL for real-time chat.
     */
    getWebSocketUrl(conversationId: string, token?: string): string {
      const baseUrl = getBaseUrl();
      const wsProtocol = baseUrl.startsWith('https') ? 'wss' : 'ws';
      const wsBaseUrl = baseUrl.replace(/^https?/, wsProtocol);
      const wsUrl = `${wsBaseUrl}/api/chat/conversations/${conversationId}/ws`;

      if (token) {
        return `${wsUrl}?token=${encodeURIComponent(token)}`;
      }
      return wsUrl;
    },
  };
}

export type ChatApi = ReturnType<typeof createChatApi>;
