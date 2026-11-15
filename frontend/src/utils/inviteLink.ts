// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The group invite link an athlete opens — one builder for every web surface that shares a code
// ABOUTME: Points at this app's /groups/join/:code, the route App.tsx reads on mount; the phone builds the same path

/** The shareable link for an invite `code`, on the origin this app is served from. */
export function inviteLink(code: string): string {
  return `${window.location.origin}/groups/join/${encodeURIComponent(code)}`;
}
