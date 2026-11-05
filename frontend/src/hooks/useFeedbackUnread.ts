// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: How many of the Dravr team's feedback replies wait unread — the account menu's dot, polled at the bell's cadence
// ABOUTME: Its own module so the app shell loads only this read, not the feedback thread's hooks and API (carnet#1392)

import { useQuery } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import { feedbackUnreadApi } from '../services/api';

/** How often the unread dot looks for a reply: the notification bell's cadence. */
const UNREAD_REFRESH_MS = 120_000;

/** How many of the team's replies are waiting unread: the account menu's dot. */
export function useFeedbackUnread(): number {
  const unread = useQuery({
    queryKey: QUERY_KEYS.user.feedbackUnread(),
    queryFn: () => feedbackUnreadApi.getUnread(),
    refetchInterval: UNREAD_REFRESH_MS,
  });
  return unread.data ?? 0;
}
