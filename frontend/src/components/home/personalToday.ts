// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The contract between Home's Today and the personal conversation beside it
// ABOUTME: What Today's sections can do to the thread, and the two shapes the host draws Today in

import type { ReactNode } from 'react';

/**
 * What Today's sections can do to the conversation beside them. Each closes
 * the Today sheet first on a narrow screen, so the athlete sees the thread
 * the action landed in; docked, the panel stays.
 */
export interface TodayActions {
  /** Put `text` in this thread's composer, for the athlete to finish and send. */
  draft: (text: string) => void;
  /**
   * Send `text` as the athlete's turn — a starter's postback, which the
   * bubble shows as `display` until the turn echoes the words it stands for.
   */
  send: (text: string, display: string) => void;
  /** Open the personal thread where one of its topics starts. */
  openTopic: (conversationId: string, dividerId: string | null) => void;
  /** Step out of the Today sheet, before something opens over the page. */
  dismiss: () => void;
}

/**
 * Home's Today, as the host draws it: the whole of it for the panel, the
 * drawer or the sheet, and the one line that stands for it above the thread
 * on a narrow screen. The chat surface only places them.
 */
export interface PersonalToday {
  /**
   * The panel's content. `actions` reach this thread from inside Today;
   * `compact` is the docked side panel, as opposed to a drawer or a sheet.
   */
  panel: (actions: TodayActions, compact: boolean) => ReactNode;
  /** The folded line; `hidden` while the athlete reads back up the thread or types. */
  peek: (onOpen: () => void, hidden: boolean) => ReactNode;
}
