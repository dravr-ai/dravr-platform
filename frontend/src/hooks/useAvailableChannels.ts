// ABOUTME: The chat apps the workspace offers for linking — one query shared by the chat-app sheet, its offers and Settings → Messaging
// ABOUTME: Always yields an array: a malformed read (an HTML error page) reads as no channels, never as a crash or a count

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useQuery } from '@tanstack/react-query';
import type { AvailableChannel } from '@pierre/api-client';
import { messagingLinkApi } from '../services/api';

/** Stable empty list so the channels value keeps identity across renders. */
const EMPTY_CHANNELS: AvailableChannel[] = [];

/** The offers (Getting started's row, a group's card) re-render often; a minute-old channel list is fine for them. */
const OFFER_STALE_MS = 60_000;

/**
 * Read the connectable channels.
 *
 * The response is defended against a non-array body (an error or HTML page
 * served with a 200): an offer must never appear on it — a string's
 * `.length` would read as a huge channel count — and the settings pane must
 * never call `.filter` on it.
 */
export function useAvailableChannels({
  enabled = true,
  staleTime = OFFER_STALE_MS,
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
