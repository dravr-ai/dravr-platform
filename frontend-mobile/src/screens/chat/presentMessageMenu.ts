// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Presents a message's long-press actions as the platform's own menu — an action sheet on iOS, a dialog on Android
// ABOUTME: Copy, share, rate up or down, and retry on the last agent turn, with selection haptics as the row opens

import type { TFunction } from '@pierre/i18n';
import { presentMenu, type MenuRow } from '../../utils/presentMenu';

/** The two ratings a message can carry; the caller toggles a repeated tap off. */
export type MessageRating = 'up' | 'down';

interface PresentMessageMenuOptions {
  /** Whether this is the last agent turn, the only one that offers a retry. */
  canRetry: boolean;
  /** The rating the message holds now; its row is marked so the athlete sees it. */
  rating: MessageRating | null;
  onCopy: () => void;
  onShare: () => void;
  /** Called with the tapped rating even when it equals `rating` — the caller toggles it off. */
  onRate: (rating: MessageRating) => void;
  onRetry: () => void;
}

/** The marked label of the rating row the message already holds. */
function rowLabel(label: string, held: boolean): string {
  return held ? `✓ ${label}` : label;
}

/**
 * Show the message menu and run whichever row the athlete picks.
 *
 * The rows are the actions the message row used to draw as its own strip of
 * icons; the long press presents them through the system's menu instead, so
 * nothing but the time sits under the prose.
 */
export function presentMessageMenu(
  { canRetry, rating, onCopy, onShare, onRate, onRetry }: PresentMessageMenuOptions,
  t: TFunction,
): void {
  const rows: MenuRow[] = [
    { label: t('common.copy'), onPress: onCopy },
    { label: t('chat.share'), onPress: onShare },
    { label: rowLabel(t('chat.feedbackGood'), rating === 'up'), onPress: () => onRate('up') },
    { label: rowLabel(t('chat.feedbackPoor'), rating === 'down'), onPress: () => onRate('down') },
  ];
  if (canRetry) {
    rows.push({ label: t('common.retry'), onPress: onRetry });
  }
  presentMenu(rows, { title: t('chat.messageActions'), cancelLabel: t('common.cancel'), haptic: true });
}
