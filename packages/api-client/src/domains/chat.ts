// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Chat domain API - conversations, history, feedback, and the single turn send
// ABOUTME: sendTurn is the ONE way any surface puts a message on the wire and reads the turn back

import type { AxiosInstance } from 'axios';
import type {
  ChatMessageAction,
  ClaimVerdict,
  CommandCatalogueResponse,
  CommandEntry,
  Conversation,
  ConversationParticipant,
  ConversationParticipantsResponse,
  Message,
  MessageFeedbackEntry,
  TurnEnvelope,
} from '@pierre/shared-types';
import type { PlatformAdapter } from '../types/platform';
import { ENDPOINTS } from '../core/endpoints';
import { parseTurnBody, TurnRequestError, type TurnCallbacks } from '../core/turn-stream';
import { readHeader, recoverFromRefusal } from '../core/auth-challenge';

// Re-export types for consumers
export type {
  ChatMessageAction,
  ClaimVerdict,
  CommandCatalogueResponse,
  CommandEntry,
  Conversation,
  ConversationParticipant,
  ConversationParticipantsResponse,
  Message,
  MessageFeedbackEntry,
  TurnEnvelope,
};

export interface ConversationsResponse {
  conversations: Conversation[];
  /** How many conversations the caller takes part in, not the page length. */
  total: number;
}

export interface MessagesResponse {
  messages: Message[];
  /**
   * The caller's own thumbs up/down feedback on the returned messages, keyed
   * by `message_id`. Absent/empty when the caller has left no feedback in the
   * conversation. Clients hydrate their per-message feedback state from this.
   */
  feedback?: MessageFeedbackEntry[];
}

/**
 * The claim verdicts attached to one conversation's messages.
 *
 * The same `claim_verdicts` rows the admin read returns; this endpoint stays
 * inside the chat domain so the chat UI can render chips and open a drawer
 * without going through the admin permission gate.
 */
export interface ChatVerdictsResponse {
  /** One row per flagged claim, newest first. */
  verdicts: ClaimVerdict[];
  /** How many rows the conversation has in total. */
  total: number;
}

/**
 * The reply's resolved scenes, when the surface draws them inline.
 *
 * The one block a client reads off the finished envelope rather than through
 * `sendTurn`'s `onBlock` walk: it goes into the persisted transcript row the
 * client builds from `onDone`, not into a panel keyed beside it.
 */
export function replySceneBlocks(turn: TurnEnvelope): string | undefined {
  const block = turn.assistant.blocks.find(b => b.type === 'scene');
  return block?.type === 'scene' ? block.scene_blocks : undefined;
}

/**
 * Everything {@link ChatApi.sendTurn} accepts beyond the message itself.
 *
 * The callbacks are the turn's only outcome channel — `sendTurn` resolves
 * either way, having called exactly one of `onDone` / `onError`.
 */
export interface SendTurnOptions extends TurnCallbacks {
  /**
   * Aborts the turn's request and drops its open body.
   *
   * The client's idle contract owns one: a tab left open with a turn still
   * streaming holds a server instance warm indefinitely, so the idle watch
   * aborts it and the athlete re-sends on their next interaction. `onError`
   * receives a sentence that says exactly that rather than the runtime's own
   * abort text.
   */
  signal?: AbortSignal;
}

/** What {@link ChatApi.sendTurn} reports when the idle watch dropped a turn. */
const ABORTED_MESSAGE =
  'The turn was stopped because the app went idle. Send it again to pick up where you left off.';

/**
 * Build the headers one turn goes out with.
 *
 * `sendTurn` is the one request in the package that bypasses the axios
 * instance — it needs the raw body to read frames from — so the interceptor's
 * work is done here instead: the bearer token where the platform stores one
 * (mobile), the CSRF token, and the client-platform header that decides which
 * in-app `SurfaceId` the turn resolves to server-side.
 */
async function turnHeaders(adapter: PlatformAdapter): Promise<Record<string, string>> {
  const headers: Record<string, string> = {
    'Content-Type': 'application/json',
    // Ask for frames. The server opens an event stream when the surface
    // declares a delta channel and answers a slash command with a single
    // JSON document regardless; one parser reads both.
    Accept: 'text/event-stream, application/json',
    'X-Client-Platform': adapter.platform,
  };
  const token = await adapter.authStorage.getToken();
  if (token) headers.Authorization = `Bearer ${token}`;
  const csrf = await adapter.authStorage.getCsrfToken();
  if (csrf) headers['X-CSRF-Token'] = csrf;
  return headers;
}

/** Turn a refused request into the error a caller can both read and format. */
async function requestFailure(response: Response): Promise<TurnRequestError> {
  const body = (await response.json().catch(() => null)) as
    | { message?: string; error?: string }
    | null;
  const message = body?.message ?? body?.error ?? `The server answered ${response.status}.`;
  return new TurnRequestError(message, response.status, body);
}

/**
 * Run the session recovery a refused turn is owed.
 *
 * `sendTurn` bypasses the axios instance, so it also bypassed the response
 * interceptor that clears a dead session and drives re-authentication — on the
 * one endpoint an athlete spends the whole session in. A credential that went
 * insufficient mid-conversation therefore drew an error bubble in the thread
 * and nothing else: no sign-out, no login screen, and the next message drew
 * the same bubble.
 *
 * Calls the same recovery the interceptor calls, rather than agreeing with it,
 * so there is no second opinion about which refusals re-authentication fixes
 * and neither path can sign an athlete out of a role they simply do not hold.
 */
async function recoverFromRefusedTurn(
  adapter: PlatformAdapter,
  response: Response,
  failure: TurnRequestError
): Promise<void> {
  await recoverFromRefusal(
    adapter,
    response.status,
    readHeader(response.headers, 'www-authenticate'),
    failure
  );
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
 *
 * The adapter comes along because one method — {@link ChatApi.sendTurn} —
 * reads a response body frame by frame, which axios cannot do in a browser.
 * It carries the platform's body reader and the auth material the axios
 * interceptors would otherwise have attached.
 */
export function createChatApi(axios: AxiosInstance, adapter: PlatformAdapter) {
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
     * List the slash commands this caller may actually run.
     *
     * The server resolves the listing per caller through the same availability
     * predicates `/help` asks each handler, so the palette a client builds from
     * this never offers a command the caller would be refused. Pass the open
     * conversation so group-scoped commands answer for the group that
     * conversation is bound to rather than the caller's first group.
     */
    async listCommands(conversationId?: string): Promise<CommandEntry[]> {
      const url = conversationId
        ? `${ENDPOINTS.COMMANDS}?conversation_id=${encodeURIComponent(conversationId)}`
        : ENDPOINTS.COMMANDS;
      const response = await axios.get<CommandCatalogueResponse>(url);
      return response.data.commands;
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
     * Who is in a conversation, owner first. 404 when the caller is not a
     * participant — the same answer a stranger gets from every conversation
     * route, so a thread's existence is never disclosed outside it.
     */
    async listParticipants(conversationId: string): Promise<ConversationParticipant[]> {
      const response = await axios.get<ConversationParticipantsResponse>(
        ENDPOINTS.CHAT.PARTICIPANTS(conversationId)
      );
      return response.data.participants;
    },

    /**
     * Add a user to a conversation the caller participates in. The user must
     * be a member of the conversation's tenant (403 otherwise); re-adding an
     * existing participant is idempotent and returns their row as it stands.
     */
    async addParticipant(
      conversationId: string,
      userId: string,
    ): Promise<ConversationParticipant> {
      const response = await axios.post<ConversationParticipant>(
        ENDPOINTS.CHAT.PARTICIPANTS(conversationId),
        { user_id: userId },
      );
      return response.data;
    },

    /**
     * Remove a member from a conversation the caller participates in. The
     * owner cannot be removed (400); a user who is not in the thread is a 404.
     */
    async removeParticipant(conversationId: string, userId: string): Promise<void> {
      await axios.delete(ENDPOINTS.CHAT.PARTICIPANT(conversationId, userId));
    },

    /**
     * Advance the caller's read marker on a conversation — up to
     * `upToMessageId`, or to the newest `user`/`assistant` row when omitted.
     * Monotonic server-side: re-marking an older row than the marker already
     * covers changes nothing, so two tabs racing cannot resurrect unread rows.
     * A caller who is not a participant gets 404.
     */
    async markConversationRead(conversationId: string, upToMessageId?: string): Promise<void> {
      await axios.post(ENDPOINTS.CHAT.READ(conversationId), {
        up_to_message_id: upToMessageId,
      });
    },

    /**
     * Clear the caller's read marker — mark the thread unread, so every
     * `user`/`assistant` row counts as unread again until it is opened.
     * Idempotent for a participant; a stranger gets 404.
     */
    async markConversationUnread(conversationId: string): Promise<void> {
      await axios.delete(ENDPOINTS.CHAT.READ(conversationId));
    },

    /**
     * Fetch the claim verdicts attached to the messages in a
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
     * Send one turn and read it back.
     *
     * The only way any surface puts a chat message on the wire. Web and
     * mobile call this same method, over the same request, through the same
     * frame parser — which is what makes a server-side capability reach both
     * clients instead of whichever one its author happened to be editing.
     *
     * The callbacks describe the turn as it unfolds: `onProgress` for tool
     * calls the pipeline observed, `onDelta` for assistant text arriving
     * ahead of the reply (only the ACP provider branch produces any), then
     * exactly one terminal callback — `onBlock` for each renderable piece of
     * the finished reply followed by `onDone`, or `onError`. It resolves
     * rather than rejecting, because the callbacks already carry the outcome.
     *
     * Everything the caller learns rides this one response body. There is no
     * second stream to open and no run id to correlate.
     */
    async sendTurn(
      conversationId: string,
      content: string,
      options?: SendTurnOptions,
    ): Promise<void> {
      let turn: TurnEnvelope;
      try {
        const response = await fetch(
          `${axios.defaults.baseURL ?? ''}${ENDPOINTS.CHAT.MESSAGES(conversationId)}`,
          {
            method: 'POST',
            headers: await turnHeaders(adapter),
            body: JSON.stringify({ content }),
            credentials: adapter.turnCredentials,
            signal: options?.signal,
          },
        );
        if (!response.ok) {
          // Recover before the throw: the caller's `onError` renders the
          // refusal, but only this path can act on it, and a caller that just
          // re-rendered a stranded session would never learn it was stranded.
          const failure = await requestFailure(response);
          await recoverFromRefusedTurn(adapter, response, failure);
          throw failure;
        }
        // `onBlock` is dispatched by the body reader, not here: a streamed
        // turn fires each block as the server decides it, and a single-JSON
        // answer walks the envelope's list. One place, both shapes.
        turn = await parseTurnBody(adapter.readBody(response), {
          onDelta: options?.onDelta,
          onProgress: options?.onProgress,
          onBlock: options?.onBlock,
        });
      } catch (error) {
        if (options?.signal?.aborted) {
          options.onError?.(new Error(ABORTED_MESSAGE));
          return;
        }
        options?.onError?.(
          error instanceof Error ? error : new Error('The turn could not be sent.'),
        );
        return;
      }

      options?.onDone?.(turn);
    },

    /**
     * Set (or update) the caller's thumbs up/down feedback on a message.
     *
     * Upsert keyed on (message, user): re-rating overwrites the prior value
     * and refreshes the optional reason. Pass `comment` to capture a
     * "what went wrong?" note (typically only on a thumbs-down). The server
     * returns 404 if the message is not in a conversation the caller owns.
     */
    async submitMessageFeedback(
      conversationId: string,
      messageId: string,
      rating: 'up' | 'down',
      comment?: string,
    ): Promise<MessageFeedbackEntry> {
      const response = await axios.post<MessageFeedbackEntry>(
        ENDPOINTS.CHAT.MESSAGE_FEEDBACK(conversationId, messageId),
        { rating, comment },
      );
      return response.data;
    },

    /**
     * Clear the caller's feedback on a message (thumbs toggle-off).
     * Idempotent — succeeds even when no feedback was stored.
     */
    async deleteMessageFeedback(conversationId: string, messageId: string): Promise<void> {
      await axios.delete(ENDPOINTS.CHAT.MESSAGE_FEEDBACK(conversationId, messageId));
    },
  };
}

export type ChatApi = ReturnType<typeof createChatApi>;
