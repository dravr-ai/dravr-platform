// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Chat domain API - conversations, history, feedback, and the single turn send
// ABOUTME: sendTurn is the ONE way any surface puts a message on the wire and reads the turn back

import { AxiosError, type AxiosInstance } from 'axios';
import type {
  AwaitingReply,
  ChatMessageAction,
  ClaimVerdict,
  CommandCatalogueResponse,
  CommandEntry,
  Conversation,
  ConversationParticipant,
  ConversationParticipantsResponse,
  Message,
  MessageFeedbackEntry,
  MessageOrigin,
  PublishSubjectPlanRequest,
  PublishSubjectPlanResult,
  StartersResponse,
  SubjectPlanResponse,
  TurnEnvelope,
  UseCaseStarter,
} from '@pierre/shared-types';
import {
  parsePlanPublishConflict,
  parsePublishedSubjectPlan,
  parseSubjectPlanResponse,
} from '@pierre/shared-types';
import type { PlatformAdapter } from '../types/platform';
import { ENDPOINTS } from '../core/endpoints';
import { requireShape } from '../core/shape';
import { readBodyProgressively } from '../core/body-reader';
import {
  parseTurnBody,
  TurnIdleAbortedError,
  TurnRequestError,
  type TurnCallbacks,
} from '../core/turn-stream';
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
  PublishSubjectPlanRequest,
  PublishSubjectPlanResult,
  SubjectPlanResponse,
  TurnEnvelope,
  UseCaseStarter,
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
  /**
   * Whether older messages remain before the first one returned. A thread is
   * read in windows from the newest back; the next window is asked for with
   * `before` set to the oldest message held.
   */
  has_more?: boolean;
  /**
   * The caller's question a turn is still answering, or `null`. Set only on
   * the newest window: a client that reloaded mid-turn shows the reply as
   * coming and re-reads the thread until it lands or this turns `null`.
   * Optional because a cache entry the client wrote itself may not carry it.
   */
  awaiting_reply?: AwaitingReply | null;
}

/** Which window of a thread to read. */
export interface MessageWindow {
  /** Messages per window (the server clamps it to 1–200; 100 when absent). */
  limit?: number;
  /** The oldest message already held: the window holds the ones before it. */
  before?: string;
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

/** What the server answers a stop with. */
export interface StopTurnResponse {
  /**
   * `true` when the running turn was closed with the stopped notice. `false`
   * when there was nothing left to stop — the reply had already landed.
   */
  stopped: boolean;
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
   * The client's idle contract owns one: a tab left alone with a turn still
   * streaming holds a server instance warm indefinitely, so the idle watch
   * aborts it once the client has gone the full idle threshold without an
   * interaction. Only the stream stops — the server finishes the turn and
   * persists its reply regardless, and the client re-reads the conversation
   * when the athlete returns. `onError` receives a sentence that says so
   * rather than the runtime's own abort text.
   */
  signal?: AbortSignal;
  /**
   * How the athlete produced the message: a pre-filled draft, a chip beside
   * their data, or typed (the default, and what an absent value means).
   */
  origin?: MessageOrigin;
}

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

/** What Home's "+" asks of the personal thread. */
export interface OpenTopicOptions {
  /** The agent the topic is with, when the athlete picked one. */
  agent_id?: string;
  /** The athlete sends the topic's first line at once (a tapped starter), so
   *  the agent's welcome, which offers the same starters, is not posted. */
  athlete_speaks_first?: boolean;
}

/** The topic `openPersonalTopic` opened. */
export interface OpenTopicResponse {
  /** The personal thread, as it stands with the topic opened. */
  conversation: Conversation;
  /** The divider row the topic opened with; absent when the current topic
   *  held no turn yet, so it already was the fresh start a divider marks. */
  divider?: Message | null;
}

export interface CreateConversationOptions {
  /** A title the athlete typed. Omitted, the server names the thread after
   *  its group, else its agent, else the moment it started, in the athlete's
   *  language — so no client invents a title of its own. */
  title?: string;
  model?: string;
  /** Coach to attach to the conversation; the coach's system_prompt
   *  is resolved server-side at runtime. */
  agent_id?: string;
  /** Coaching group to scope the conversation to. When set, the
   *  server-side prompt-assembly stage injects group context (member
   *  roster, peer training data subject to per-member consent,
   *  role-aware summaries). The caller must be an active member of
   *  the group; the REST endpoint rejects non-members with 403. */
  group_id?: string;
  /** The thread is created to carry the athlete's first line at once (a
   *  composer that creates its thread on send, a prompt queued before any
   *  thread existed). The agent bound to it then posts no welcome — the
   *  athlete spoke first, and its first reply introduces it. */
  athlete_speaks_first?: boolean;
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
     * The use-case starters an empty thread offers, ranked from the athlete's
     * state for the agent a new thread would be bound to — at most three.
     *
     * Each read counts as one showing of them, which is how the server
     * retires a starter the athlete keeps passing over: ask once per time
     * the empty thread is drawn, not on every focus.
     */
    async listStarters(): Promise<UseCaseStarter[]> {
      const response = await axios.get<StartersResponse>(ENDPOINTS.CHAT.STARTERS);
      return response.data.starters;
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
     * The caller's personal thread: the one continuous conversation Home
     * opens. The server resolves it — adopting the athlete's newest thread of
     * their own the first time — so every client and every server job agrees
     * on which thread it is. `null` for an athlete who has none yet: nothing
     * is forged for a look, and "+" or a starter opens it.
     */
    async getPersonalConversation(): Promise<Conversation | null> {
      const response = await axios.get<{ conversation: Conversation | null }>(
        ENDPOINTS.CHAT.PERSONAL
      );
      return response.data.conversation ?? null;
    },

    /**
     * Open a new topic in the personal thread — Home's "+". A divider row,
     * after which the coach reads the thread afresh; with `agent_id` the
     * thread is rebound to that agent, whose welcome opens the topic. No
     * conversation is created, so no cap applies.
     */
    async openPersonalTopic(options?: OpenTopicOptions): Promise<OpenTopicResponse> {
      const response = await axios.post<OpenTopicResponse>(
        ENDPOINTS.CHAT.PERSONAL_TOPICS,
        options ?? {}
      );
      return response.data;
    },

    /**
     * Open the caller's plan thread for one athlete of a group they coach —
     * the one thread per coach per athlete where the coach works on that
     * athlete's plan (carnet#1070). The server checks the coaching link by id
     * (the caller is the group's coach, the athlete a live member who shares
     * with them, one home tenant) and answers the existing thread, un-archived,
     * or a new one bound to the group's agent. A failed check is a refusal
     * (403 / 404 / 409), never a thread.
     */
    async openAthleteThread(athleteUserId: string, groupId: string): Promise<Conversation> {
      const response = await axios.post<Conversation>(ENDPOINTS.CHAT.ATHLETE_THREAD(athleteUserId), {
        group_id: groupId,
      });
      return response.data;
    },

    /**
     * The plan thread's athlete as the coach reads them: their live plan and
     * the coach's draft over it, with where that draft stands. 404 for a
     * thread bound to no athlete; the coaching-link refusals otherwise.
     */
    async getSubjectPlan(conversationId: string): Promise<SubjectPlanResponse> {
      const endpoint = ENDPOINTS.CHAT.SUBJECT_PLAN(conversationId);
      const response = await axios.get<unknown>(endpoint);
      return requireShape(parseSubjectPlanResponse(response.data), endpoint);
    },

    /**
     * Publish the coach's draft as the athlete's live plan; the athlete is
     * notified. The athlete's plan having moved under the draft — a week both
     * changed, or a re-laid season — is answered 409 `plan_publish_conflict`,
     * which resolves as a `conflict` result rather than rejecting: the coach
     * decides what happens next, and `overwrite_athlete_changes` is how they
     * publish anyway. Every other refusal rejects as usual.
     */
    async publishSubjectPlan(
      conversationId: string,
      request: PublishSubjectPlanRequest,
    ): Promise<PublishSubjectPlanResult> {
      const endpoint = ENDPOINTS.CHAT.SUBJECT_PLAN_PUBLISH(conversationId);
      const response = await axios.post<unknown>(endpoint, request, {
        validateStatus: (status) => (status >= 200 && status < 300) || status === 409,
      });
      if (response.status === 409) {
        const conflict = parsePlanPublishConflict(response.data);
        if (conflict === null) {
          // A 409 that is not the conflict is a refusal like any other, and
          // rejects in the shape every caller already classifies.
          throw new AxiosError(
            `${endpoint} refused with 409`,
            AxiosError.ERR_BAD_REQUEST,
            response.config,
            response.request,
            response,
          );
        }
        return { status: 'conflict', conflict };
      }
      return { status: 'published', published: requireShape(parsePublishedSubjectPlan(response.data), endpoint) };
    },

    /**
     * Throw the coach's draft away. The athlete's live plan stays as it is
     * and nothing reaches them. 404 `no_draft` when there is none.
     */
    async discardSubjectPlanDraft(conversationId: string): Promise<void> {
      await axios.delete(ENDPOINTS.CHAT.SUBJECT_PLAN_DRAFT(conversationId));
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
     * Get one window of a conversation's messages, oldest first: the newest
     * by default, or the one before `window.before`.
     */
    async getConversationMessages(
      conversationId: string,
      window: MessageWindow = {}
    ): Promise<MessagesResponse> {
      const params: Record<string, string | number> = {};
      if (window.limit !== undefined) params.limit = window.limit;
      if (window.before) params.before = window.before;
      const response = await axios.get<MessagesResponse>(
        ENDPOINTS.CHAT.MESSAGES(conversationId),
        { params }
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
     * ahead of the reply (the Copilot ACP and Anthropic API heads produce
     * it) and `onRetract` when that text turns out not to be the reply, then
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
            body: JSON.stringify(
              options?.origin ? { content, origin: options.origin } : { content },
            ),
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
        turn = await parseTurnBody(readBodyProgressively(response), {
          onDelta: options?.onDelta,
          onRetract: options?.onRetract,
          onProgress: options?.onProgress,
          onBlock: options?.onBlock,
        });
      } catch (error) {
        if (options?.signal?.aborted) {
          options.onError?.(new TurnIdleAbortedError());
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
     * Stop the turn answering the caller's latest message.
     *
     * Dropping the stream never stops a turn — the server finishes it and
     * stores the reply regardless — so this is the only way to end one. The
     * server closes the question with a short "stopped" notice and the open
     * `sendTurn` stream ends on `onDone` carrying that notice as the
     * assistant's message, so the caller renders a stopped turn through the
     * path it already renders every turn with.
     *
     * @returns whether a running turn was stopped; `false` when the reply had
     *   already landed and the stream is about to deliver it.
     */
    async stopTurn(conversationId: string): Promise<boolean> {
      const response = await axios.post<StopTurnResponse>(ENDPOINTS.CHAT.STOP(conversationId));
      return response.data.stopped;
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
