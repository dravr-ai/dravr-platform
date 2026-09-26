// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The web client's binding of the shared notification hooks — its API and its unread-badge poll
// ABOUTME: The hooks themselves live in @pierre/ui-logic, one definition for web and mobile

import { createNotificationHooks } from '@pierre/ui-logic';
import { notificationsApi } from '../services/api';

export const {
  useNotificationFeed,
  useUnreadCount,
  useNotificationActions,
  useNotificationPreferences,
} = createNotificationHooks(notificationsApi, {
  // A low-value background counter: polled every two minutes, and not
  // refetched on window focus or mount, because tab-hopping was generating
  // 80+ requests per session.
  staleTime: 60_000,
  refetchInterval: 120_000,
  refetchOnWindowFocus: false,
  refetchOnMount: false,
});
