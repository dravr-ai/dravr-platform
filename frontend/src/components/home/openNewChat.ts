// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: What every web "new chat" does — a new conversation beside the athlete's personal thread, or that thread when they have none — and how a Your team row reopens its agent's latest chat first
// ABOUTME: The personal thread is never rebound to another agent nor split by a divider from here (carnet#1473)

import type { QueryClient } from '@tanstack/react-query';
import type { Conversation } from '@pierre/shared-types';
import { chatApi } from '../../services/api';
import { QUERY_KEYS } from '../../constants/queryKeys';

/** What a "new chat" asks for. */
export interface NewChatRequest {
  /** The agent the chat is with, when the athlete picked one; omitted, the server binds their selected agent. */
  agent_id?: string;
  /** The athlete sends the chat's first line at once, so the agent posts no welcome over it. */
  athlete_speaks_first?: boolean;
}

/** The conversation a "new chat" opened. */
export interface OpenedChat {
  id: string;
  /**
   * The divider the chat opened with, when it was the athlete's first and so
   * became their personal thread as a topic; `null` for a new conversation,
   * which is read from its start.
   */
  dividerId: string | null;
}

/**
 * Open a new chat for the athlete (carnet#1473).
 *
 * An athlete who already has a personal thread gets a new conversation of
 * its own — bound to the agent named, named by the server, welcomed by the
 * agent unless the athlete speaks first, and counted against the active
 * conversation cap (a 429 naming `max_active_conversations` at the cap). The
 * personal thread is left as it was, so each agent keeps its own thread and
 * the earlier ones stay in History.
 *
 * An athlete with none yet has their first chat become the personal thread:
 * the topic endpoint forges it.
 *
 * The personal thread is read through the cache, so a click made while Home
 * is still resolving it waits for that answer instead of reading "none".
 */
export async function openNewChat(queryClient: QueryClient, request: NewChatRequest): Promise<OpenedChat> {
  const personal = await queryClient.fetchQuery({
    queryKey: QUERY_KEYS.chat.personal(),
    queryFn: () => chatApi.getPersonalConversation(),
  });

  if (personal) {
    const created = await chatApi.createConversation(request);
    void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() });
    return { id: created.id, dividerId: null };
  }

  return openPersonalFirstChat(queryClient, request);
}

/**
 * How many of the athlete's newest conversations {@link openChatWithAgent}
 * looks through: the server's widest list page, which also bounds the
 * highest active-conversation cap an admin can set.
 */
const AGENT_CHAT_LOOKUP_LIMIT = 200;

/** When a conversation was last spoken in, for ordering; its last message, else its last update. */
function lastActive(conversation: Conversation): number {
  return Date.parse(conversation.last_message_at ?? conversation.updated_at) || 0;
}

/**
 * Open the athlete's chat with one agent, the way a messenger opens a
 * contact: their most recent own 1:1 conversation bound to that agent —
 * the personal thread included — else a new one (carnet#1473).
 *
 * "+" is what always starts a new conversation; a tap on an agent the
 * athlete already talks to lands back in that thread. Only the athlete's
 * newest conversations are looked through, so one with the agent older than
 * all of them yields a new chat.
 */
export async function openChatWithAgent(queryClient: QueryClient, agentId: string): Promise<OpenedChat> {
  const { conversations } = await chatApi.getConversations(AGENT_CHAT_LOOKUP_LIMIT);
  const latest = conversations
    .filter(
      (conversation) =>
        conversation.agent_id === agentId &&
        !conversation.group_id &&
        !conversation.subject_user_id &&
        !conversation.archived_at,
    )
    .reduce<Conversation | null>(
      (best, conversation) => (best === null || lastActive(conversation) > lastActive(best) ? conversation : best),
      null,
    );
  if (latest) return { id: latest.id, dividerId: null };
  return openNewChat(queryClient, { agent_id: agentId });
}

/** An athlete's first chat: the topic endpoint forges their personal thread. */
async function openPersonalFirstChat(queryClient: QueryClient, request: NewChatRequest): Promise<OpenedChat> {
  const { conversation, divider } = await chatApi.openPersonalTopic(request);
  void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() });
  void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.messages(conversation.id) });
  void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.personal() });
  return { id: conversation.id, dividerId: divider?.id ?? null };
}
