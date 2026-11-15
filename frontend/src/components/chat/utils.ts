// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Re-exports shared utilities for chat components
// ABOUTME: Sources from @pierre/domain-utils and @pierre/chat-utils packages

// OAuth-aware URL formatting
export { getFriendlyUrlName, linkifyUrls } from '@pierre/domain-utils';

/** The element id of a topic's divider, which History's topic list scrolls to. */
export function topicAnchorId(dividerId: string): string {
  return `topic-${dividerId}`;
}
