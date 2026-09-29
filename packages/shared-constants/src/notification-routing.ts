// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Turns a notification's server-declared `data.screen` into each platform's own route
// ABOUTME: One vocabulary, one resolver — the two hand-written client maps this replaces are gone

import {
  NOTIFICATION_SCREEN_SURFACES,
  type NotificationScreen,
} from './surface-capabilities.generated';
import { destinationRoutes, type DestinationRoutes } from './surfaces';

/**
 * The surface a notification opens.
 *
 * Web and mobile each used to carry a `switch` over the same screen names,
 * in their own file, returning their own route shape. Nothing checked
 * that the two agreed, and nothing checked either against what the server
 * emits — so `connections`, which the provider-reauth notification has always
 * sent, matched neither map and tapping it navigated nowhere on both
 * platforms.
 *
 * There is one map now and the server writes it: `NOTIFICATION_SCREEN_SURFACES`
 * is generated from the server's own `NotificationScreen` enum, and it names a
 * destination rather than a route — a top-level surface or a settings pane —
 * because the registry already holds each platform's route for either.
 */
export interface NotificationDestination {
  /** Where the destination the notification points at is served. */
  routes: DestinationRoutes;
  /**
   * The thread a `coach` payload names in `id`: the conversation an agent
   * message was sent in, or the one where the agent computed the insight the
   * notification reports. The server sends
   * `{ screen: "coach", action: "chat", id: <conversation_id> }` for both.
   */
  conversationId?: string;
}

/** Read a value as a screen name the server declares, or null. */
function asScreen(value: unknown): NotificationScreen | null {
  return typeof value === 'string' && value in NOTIFICATION_SCREEN_SURFACES
    ? (value as NotificationScreen)
    : null;
}

/**
 * Resolve where a notification should take the athlete.
 *
 * A notification's action buttons open the notification's own destination —
 * an agent message's "Reply" its thread, a sync failure's "Reconnect" the
 * connections pane — so the payload alone decides.
 *
 * Returns null when the payload names nowhere to go: no screen, a screen this
 * vocabulary does not carry (a row stored before a token was retired), or the
 * chat without a thread. The clients render such a notification as
 * information rather than as a link, so nothing looks tappable and leads
 * nowhere.
 */
export function resolveNotificationDestination(
  data: Record<string, unknown> | null | undefined,
): NotificationDestination | null {
  const screen = asScreen(data?.screen);
  if (!screen) return null;

  const surface = NOTIFICATION_SCREEN_SURFACES[screen];
  const routes = destinationRoutes(surface);
  if (!routes) return null;

  // Only the coach screen names a conversation in `id`; a personal record's
  // `id` is its activity, and reading it as a thread would open a
  // conversation that does not exist.
  const conversationId = screen === 'coach' && typeof data?.id === 'string' ? data.id : null;
  if (conversationId !== null) return { routes, conversationId };

  // The chat is a list of threads. A notification that names none has nothing
  // there to show, and opening the empty chat is the dead end this refuses.
  return surface === 'chat' ? null : { routes };
}

/**
 * The Dashboard route a notification opens on web: a `tab`, or `tab/subview`
 * for an agent message that names its conversation.
 */
export function webNotificationRoute(
  data: Record<string, unknown> | null | undefined,
): string | null {
  const destination = resolveNotificationDestination(data);
  const web = destination?.routes.web;
  if (!web) return null;

  return destination.conversationId
    ? `${web}/${encodeURIComponent(destination.conversationId)}`
    : web;
}

/**
 * The thread route on the phone. It lives beside the tabs rather than under
 * the chat tab: pushed in the app stack it covers the tab bar, so the
 * composer is the only chrome at the bottom of a discussion. The app's
 * `CHAT_THREAD_ROUTE` is this value, so a notification and a row open the
 * same screen.
 */
export const MOBILE_THREAD_PATHNAME = '/(app)/chat/[conversationId]' as const;

/** An expo-router navigation target: a grouped pathname plus optional params. */
export interface NotificationNavTarget {
  /**
   * The grouped pathname, e.g. `/(app)/(tabs)/(chat)` — or, for an agent
   * message that names its conversation, the thread route,
   * {@link MOBILE_THREAD_PATHNAME}.
   */
  pathname: string;
  /** Route params, e.g. the conversation an agent message reopens. */
  params?: Record<string, string>;
}

/**
 * The expo-router target a notification opens on mobile.
 *
 * The app's routes live under nested route groups, so a notification must
 * target the full grouped path the registry holds — `router.push('/coach')`
 * resolves to nothing.
 */
export function mobileNotificationTarget(
  data: Record<string, unknown> | null | undefined,
): NotificationNavTarget | null {
  const destination = resolveNotificationDestination(data);
  const pathname = destination?.routes.mobile;
  if (!pathname) return null;

  // A conversation opens the thread route, which sits beside the tabs, not
  // the chat tab's list.
  const { conversationId } = destination;
  return conversationId
    ? { pathname: MOBILE_THREAD_PATHNAME, params: { conversationId } }
    : { pathname };
}
