// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Maps the `screen` field on notification payloads to frontend tab IDs
// ABOUTME: Source of truth for notification → Dashboard route translation

/**
 * Translate a notification's `data.screen` value into the Dashboard tab id
 * that should become active when the notification is clicked.
 *
 * The backend triggers (`dravr-commere/src/triggers.rs`) emit a small set
 * of screen names — `recovery`, `activity`, `activities`, `stats`,
 * `social`, `coach`, `settings` — that don't 1:1 map to the web app's
 * tab ids. This helper bridges them. Returns `null` when the screen
 * name has no useful target on web (the click should still mark the
 * notification read but not navigate).
 *
 * Mobile uses the screen names directly via React Navigation, so the
 * payloads stay platform-neutral on the wire.
 */
export function mapScreenToTab(screen: string | undefined | null): string | null {
  if (!screen) return null;
  switch (screen) {
    // Health/recovery dashboards live under Insights on the web build.
    case 'recovery':
    case 'activity':
    case 'activities':
    case 'stats':
      return 'insights';
    // Social hub (friends, kudos, social feed) is the Insights tab's
    // friends sub-view; for now we still surface it under `insights`
    // and the panel's caller decides whether to flip the sub-view.
    case 'social':
      return 'insights';
    // Coach messages reopen the chat tab.
    case 'coach':
      return 'chat';
    // Settings / re-auth deep links open the settings tab.
    case 'settings':
      return 'settings';
    default:
      return null;
  }
}

/**
 * Resolve the full Dashboard route (a `tab` or `tab/subview` string) that a
 * notification should open when clicked, given its `data` payload and the
 * optional id of the pressed action button.
 *
 * Coach-message notifications carry the originating conversation id on
 * `data.id` (`dravr-commere` `trigger_coach_message`:
 * `{ screen: "coach", action: "chat", id: <conversation_id> }`). Reading
 * only the `screen` field routed the "Reply" button to the bare `chat`
 * tab, which strands the athlete on the empty "Ready to analyze your
 * fitness" coach picker instead of the thread they were replying to (web
 * QA 2026-07-13). Emitting `chat/<conversation_id>` lets the Dashboard's
 * hash router select that conversation directly.
 *
 * Returns `null` when the payload has no useful web target (the click
 * still marks the notification read but does not navigate).
 */
export function resolveNotificationRoute(
  data: Record<string, unknown> | undefined,
  actionId?: string,
): string | null {
  const screen = typeof data?.screen === 'string' ? data.screen : undefined;
  const tab = mapScreenToTab(screen) ?? (actionId ? mapScreenToTab(actionId) : null);
  if (!tab) return null;
  // Coach messages deep-link to their specific conversation thread.
  if (tab === 'chat') {
    const conversationId = typeof data?.id === 'string' ? data.id : undefined;
    if (conversationId) {
      return `chat/${encodeURIComponent(conversationId)}`;
    }
  }
  return tab;
}
