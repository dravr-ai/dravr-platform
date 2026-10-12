// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The use-case starters the phone's empty thread offers, ranked by the server from the athlete's state
// ABOUTME: Read once each time the empty thread is drawn — the server counts each read as one showing (carnet#829)

import { useQuery } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { UseCaseStarter } from '@pierre/shared-types';
import { chatApi } from '../services/api';

/**
 * The starters to draw while `shown` — the thread is empty and on screen.
 *
 * Asked again each time the empty thread comes back, and never on a return
 * to the app or a reconnect, which would count a showing nobody looked at.
 * Empty while they load, when the read fails, or when none fits: the thread
 * then keeps its one line and the slash hint.
 */
export function useEmptyThreadStarters(shown: boolean): UseCaseStarter[] {
  const query = useQuery({
    queryKey: QUERY_KEYS.chat.starters(),
    queryFn: () => chatApi.listStarters(),
    enabled: shown,
    staleTime: 0,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
    retry: false,
  });
  return query.data ?? [];
}
