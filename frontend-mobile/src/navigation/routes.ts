// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The expo-router paths the chat tab and the agent edit sheet live at, in one place
// ABOUTME: Screens, the tab bar, deep links and tests read these so a moved route changes one line

import { MOBILE_THREAD_PATHNAME } from '@pierre/shared-constants';

/** The chat tab: the conversation list, and where the app lands after onboarding. */
export const CHAT_LIST_ROUTE = '/(app)/(tabs)/(chat)' as const;

/**
 * One thread; `conversationId` is a stored id or {@link NEW_CONVERSATION_ID}.
 *
 * The thread lives beside the tabs, not inside the chat tab: pushed in the
 * app stack it covers the tab bar, so the composer is the only chrome at the
 * bottom of a discussion. The value is the shared one a notification targets,
 * so a push and a row cannot open different screens.
 */
export const CHAT_THREAD_ROUTE = MOBILE_THREAD_PATHNAME;

/** The `conversationId` that opens an empty composer instead of a stored thread. */
export const NEW_CONVERSATION_ID = 'new';

/**
 * What a navigation may put in the thread's composer on arrival.
 *
 * `draft` fills the composer and leaves the athlete to press send — the shape
 * a hint or a suggestion takes. `send` fills it and sends it once the thread
 * exists, which is how an invite link or a "New group chat" prompt runs its
 * command without the athlete retyping it. Both carry command text built by
 * `COMMAND_DRAFTS`, never a hand-spelled command.
 */
export interface ComposerIntent {
  draft?: string;
  send?: string;
}

/** A navigation target for one thread, as `router.push` takes it. */
export interface ThreadHref {
  pathname: typeof CHAT_THREAD_ROUTE;
  params: { conversationId: string; draft?: string; send?: string };
}

/** The target that opens `conversationId`, or a fresh composer when omitted. */
export function threadHref(
  conversationId: string = NEW_CONVERSATION_ID,
  composer?: ComposerIntent,
): ThreadHref {
  const params: ThreadHref['params'] = { conversationId };
  if (composer?.draft) params.draft = composer.draft;
  if (composer?.send) params.send = composer.send;
  return { pathname: CHAT_THREAD_ROUTE, params };
}

/**
 * The edit sheet for one of the athlete's own agents, under Discover. The
 * only agent editor in the app: agent creation is the `/agent create` command.
 */
export const COACH_EDIT_ROUTE = '/(app)/(tabs)/(discover)/edit/[agentId]' as const;
