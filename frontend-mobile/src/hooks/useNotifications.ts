// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The mobile client's binding of the shared notification hooks — its API and its unread-badge poll
// ABOUTME: The hooks themselves live in @pierre/ui-logic, one definition for web and mobile

import { createNotificationHooks } from '@pierre/ui-logic';
import { notificationsApi } from '../services/api';

export const {
  useNotificationFeed,
  useUnreadCount,
  useNotificationActions,
  useNotificationPreferences,
} = createNotificationHooks(notificationsApi, {
  // Polled every minute for the badge; focus and mount follow the client's
  // QueryClient defaults.
  staleTime: 30_000,
  refetchInterval: 60_000,
});
