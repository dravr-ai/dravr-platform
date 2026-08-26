// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The expo-router paths the chat tab and the coach library live at, in one place
// ABOUTME: Screens, the tab bar, deep links and tests read these so a moved route changes one line

/** The chat tab: the conversation list, and where the app lands after onboarding. */
export const CHAT_LIST_ROUTE = '/(app)/(tabs)/(chat)' as const;

/** One thread; `conversationId` is a stored id or {@link NEW_CONVERSATION_ID}. */
export const CHAT_THREAD_ROUTE = '/(app)/(tabs)/(chat)/[conversationId]' as const;

/** The `conversationId` that opens an empty composer instead of a stored thread. */
export const NEW_CONVERSATION_ID = 'new';

/** A navigation target for one thread, as `router.push` takes it. */
export interface ThreadHref {
  pathname: typeof CHAT_THREAD_ROUTE;
  params: { conversationId: string };
}

/** The target that opens `conversationId`, or a fresh composer when omitted. */
export function threadHref(conversationId: string = NEW_CONVERSATION_ID): ThreadHref {
  return { pathname: CHAT_THREAD_ROUTE, params: { conversationId } };
}

/** The coach library — installed coaches with their filters, import and export — under Discover. */
export const COACH_LIBRARY_ROUTE = '/(app)/(tabs)/(discover)/library' as const;

/** One installed coach's detail page. */
export const COACH_DETAIL_ROUTE = '/(app)/(tabs)/(discover)/library/[coachId]' as const;

/** The coach editor: create when called without `coachId`, edit when with. */
export const COACH_EDITOR_ROUTE = '/(app)/(tabs)/(discover)/library/editor' as const;

/** The Groups tab. */
export const GROUPS_ROUTE = '/(app)/(tabs)/(groups)' as const;
