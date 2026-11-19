// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Reads the query string of a URL an auth session hands back to the app, fragment dropped
// ABOUTME: Shared by the hosted sign-in and the Slack/Discord channel link, which both return on a dravr:// callback

/** The query string of a callback URL, without its fragment. */
export function callbackQuery(url: string): string {
  const withoutFragment = url.split('#')[0];
  const queryStart = withoutFragment.indexOf('?');
  return queryStart === -1 ? '' : withoutFragment.slice(queryStart + 1);
}
