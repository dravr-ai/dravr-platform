// ABOUTME: The chat apps the workspace offers for linking — one query shared by onboarding and Settings → Messaging
// ABOUTME: Always yields an array: a malformed read (an HTML error page) reads as no channels, never as a crash or a count

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useQuery } from '@tanstack/react-query';
import type { AvailableChannel } from '@pierre/api-client';
import { messagingLinkApi } from '../services/api';

/** Stable empty list so the channels value keeps identity across renders. */
const EMPTY_CHANNELS: AvailableChannel[] = [];

/** Onboarding re-evaluates its steps on every render; a minute-old channel list is fine for that. */
const ONBOARDING_STALE_MS = 60_000;

/**
 * Read the connectable channels.
 *
 * The response is defended against a non-array body (an error or HTML page
 * served with a 200): onboarding must never intercept the flow on it — a
 * string's `.length` would read as a huge channel count — and the settings
 * pane must never call `.filter` on it.
 */
export function useAvailableChannels({
  enabled = true,
  staleTime = ONBOARDING_STALE_MS,
}: {
  enabled?: boolean;
  /** How old a cached read may be before a newly mounted reader asks again. */
  staleTime?: number;
} = {}) {
  const query = useQuery({
    queryKey: ['messaging-available-channels'],
    queryFn: () => messagingLinkApi.getAvailableChannels(),
    enabled,
    staleTime,
  });
  const channels = Array.isArray(query.data) ? query.data : EMPTY_CHANNELS;
  return { channels, query };
}
