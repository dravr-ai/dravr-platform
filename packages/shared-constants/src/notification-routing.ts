// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Turns a notification's server-declared `data.screen` into each platform's own route
// ABOUTME: One vocabulary, one resolver — the two hand-written client maps this replaces are gone

import {
  NOTIFICATION_SCREEN_SURFACES,
  type NotificationScreen,
} from './surface-capabilities.generated';
import { surfaceById, type UserSurface } from './surfaces';

/**
 * The surface a notification opens.
 *
 * Web and mobile each used to carry a `switch` over the same seven screen
 * names, in their own file, returning their own route shape. Nothing checked
 * that the two agreed, and nothing checked either against what the server
 * emits — so `connections`, which the provider-reauth notification has always
 * sent, matched neither map and tapping it navigated nowhere on both
 * platforms.
 *
 * There is one map now and the server writes it: `NOTIFICATION_SCREEN_SURFACES`
 * is generated from the server's own `NotificationScreen` enum, and it names a
 * surface rather than a route, because `USER_SURFACES` already holds each
 * platform's route for a surface.
 */
export interface NotificationDestination {
  /** The registry surface the notification points at. */
  surface: UserSurface;
  /**
   * Conversation to preselect, for an agent message that carries one.
   *
   * `dravr-commere`'s `trigger_coach_message` sends
   * `{ screen: "coach", action: "chat", id: <conversation_id> }`. Routing on
   * `screen` alone strands the athlete on the chat surface with no thread
   * open instead of the thread they were replying to.
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
 * Falls back to the pressed action's id when the payload carries no usable
 * `screen`, which is how an action button labelled "Settings" routes on a
 * payload that only named the notification's own subject. Returns null when
 * neither names a screen — the tap still marks the notification read, it just
 * does not navigate.
 */
export function resolveNotificationDestination(
  data: Record<string, unknown> | null | undefined,
  actionId?: string,
): NotificationDestination | null {
  const screen = asScreen(data?.screen) ?? asScreen(actionId);
  if (!screen) return null;

  const surface = surfaceById(NOTIFICATION_SCREEN_SURFACES[screen]);
  if (!surface) return null;

  // Only the coach screen names a conversation in `id`. Every training screen
  // (activity, recovery, stats …) also opens the chat surface now that the
  // Insights tab is gone, but their `id` is the activity or alert itself —
  // reading it as a thread would open a conversation that does not exist.
  if (screen === 'coach' && typeof data?.id === 'string') {
    return { surface, conversationId: data.id };
  }
  return { surface };
}

/**
 * The Dashboard route a notification opens on web: a `tab`, or `tab/subview`
 * for an agent message that names its conversation.
 */
export function webNotificationRoute(
  data: Record<string, unknown> | null | undefined,
  actionId?: string,
): string | null {
  const destination = resolveNotificationDestination(data, actionId);
  if (!destination?.surface.web) return null;

  const { surface, conversationId } = destination;
  return conversationId
    ? `${surface.web}/${encodeURIComponent(conversationId)}`
    : (surface.web as string);
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
  actionId?: string,
): NotificationNavTarget | null {
  const destination = resolveNotificationDestination(data, actionId);
  if (!destination?.surface.mobile) return null;

  const { surface, conversationId } = destination;
  const pathname = surface.mobile as string;
  // The chat tab lands on the conversation list since the Chat-First Cutover;
  // a named conversation opens the thread route, not the list.
  return conversationId
    ? { pathname: MOBILE_THREAD_PATHNAME, params: { conversationId } }
    : { pathname };
}
