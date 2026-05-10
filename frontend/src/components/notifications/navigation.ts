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
