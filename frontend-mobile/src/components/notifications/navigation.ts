// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Maps notification `data.screen` payloads to expo-router navigation targets
// ABOUTME: Source of truth for notification → mobile screen routing (coach deep-links to its chat)

/**
 * Expo-router grouped pathnames for the tabs a notification can open. The
 * app's routes live under nested route groups (`(app)/(tabs)/(chat)` …),
 * so notifications must target the full grouped path — `router.push('/coach')`
 * resolves to nothing. The `(social)` group is the "Insights" tab.
 */
const CHAT_ROUTE = '/(app)/(tabs)/(chat)';
const INSIGHTS_ROUTE = '/(app)/(tabs)/(social)';
const SETTINGS_ROUTE = '/(app)/(tabs)/(settings)';

/** An expo-router navigation target: a grouped pathname plus optional params. */
export interface NotificationNavTarget {
  pathname: string;
  params?: Record<string, string>;
}

/**
 * Translate a notification's `data.screen` value into the mobile screen it
 * should open. Mirrors the web `mapScreenToTab` targets: the backend
 * triggers (`dravr-commere/src/triggers.rs`) emit `recovery`, `activity`,
 * `activities`, `stats`, `social`, `coach`, `settings`. Returns `null` for
 * screen names with no mobile destination (the tap still marks the
 * notification read but does not navigate).
 */
function mapScreenToTarget(screen: string | undefined | null): NotificationNavTarget | null {
  if (!screen) return null;
  switch (screen) {
    // Health / recovery / activity dashboards and the social hub (friends,
    // kudos, feed) all live under the Insights tab.
    case 'recovery':
    case 'activity':
    case 'activities':
    case 'stats':
    case 'social':
      return { pathname: INSIGHTS_ROUTE };
    // Coach messages reopen the chat tab.
    case 'coach':
      return { pathname: CHAT_ROUTE };
    // Settings / re-auth deep links open the settings tab.
    case 'settings':
      return { pathname: SETTINGS_ROUTE };
    default:
      return null;
  }
}

/**
 * Resolve the expo-router target a notification should open when tapped,
 * given its `data` payload and the optional id of the pressed action button.
 *
 * Coach-message notifications carry the originating conversation id on
 * `data.id` (`dravr-commere` `trigger_coach_message`:
 * `{ screen: "coach", action: "chat", id: <conversation_id> }`). Routing on
 * `screen` alone sent the "Reply" action to a non-existent `/coach` route
 * with the id under the wrong param key, so the reply never opened the
 * thread (parity with the web QA 2026-07-13 fix). Emitting the chat route
 * with `conversationId` lets `ChatScreen` select that conversation, matching
 * how the conversations list already navigates.
 *
 * Returns `null` when the payload has no usable mobile target.
 */
export function resolveNotificationTarget(
  data: Record<string, unknown> | null | undefined,
  actionId?: string,
): NotificationNavTarget | null {
  const screen = typeof data?.screen === 'string' ? data.screen : undefined;
  const target = mapScreenToTarget(screen) ?? (actionId ? mapScreenToTarget(actionId) : null);
  if (!target) return null;
  // Coach messages deep-link to their specific conversation thread.
  if (target.pathname === CHAT_ROUTE) {
    const conversationId = typeof data?.id === 'string' ? data.id : undefined;
    if (conversationId) {
      return { pathname: CHAT_ROUTE, params: { conversationId } };
    }
  }
  return target;
}
