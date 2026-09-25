// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The expo-router paths the Home and chat tabs and the agent edit sheet live at, in one place
// ABOUTME: Screens, the tab bar, deep links and tests read these so a moved route changes one line

import { MOBILE_THREAD_PATHNAME, surfaceById } from '@pierre/shared-constants';

/** The chat tab: the conversation list. */
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

/**
 * The mobile route of a surface the shared registry declares this app serves.
 *
 * Throws at module load rather than returning null: the registry is static
 * data in this monorepo, and `SurfaceParity.test.ts` already fails when a
 * surface declared for mobile has no screen, so a missing row is a build
 * error, not a runtime state to branch on.
 */
function mobileRouteOf(id: string): string {
  const route = surfaceById(id)?.mobile;
  if (route === undefined || route === null) {
    throw new Error('surface registry declares no mobile route for ' + id);
  }
  return route;
}

/**
 * The Home tab — today's session, the week around it, the latest activities —
 * where the app lands after sign-in and onboarding, and where the Dravr
 * lockup leads from any header that carries it. Read from the surface
 * registry, so the tab the app lands on and the route the registry declares
 * for mobile cannot differ.
 */
export const HOME_ROUTE = mobileRouteOf('home');

/**
 * The connected-apps screen — the external MCP clients the athlete approved.
 * Account's connected-apps row and the Connections pane's section both push
 * it, and both read the path from the surface registry, so the route a screen
 * opens and the route the registry declares for mobile cannot differ.
 */
export const CONNECTED_APPS_ROUTE = mobileRouteOf('connected-apps');

/**
 * The connections pane — the athlete's fitness providers. Group info sends a
 * coach here when their own TrainingPeaks is missing, dead or behind the
 * current notice, the three things only that pane fixes.
 */
export const CONNECTIONS_ROUTE = mobileRouteOf('data-providers');
