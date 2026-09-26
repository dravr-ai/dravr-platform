// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Presents a remembered fact's actions as the platform's own menu — an action sheet on iOS, a dialog on Android
// ABOUTME: Forget is the one row and it is destructive; selection haptics fire as the menu opens

import type { TFunction } from '@pierre/i18n';
import { presentMenu } from '../../utils/presentMenu';

interface PresentMemoryFactMenuOptions {
  onForget: () => void;
}

/**
 * Show a fact's menu and run whichever row the athlete picks.
 *
 * Forgetting is the only thing to do with a fact, so the menu is one
 * destructive row and cancel; the system draws it, and the fact row itself
 * carries no button.
 */
export function presentMemoryFactMenu({ onForget }: PresentMemoryFactMenuOptions, t: TFunction): void {
  presentMenu([{ label: t('shell.memoryForget'), onPress: onForget }], {
    title: t('shell.memoryForgetConfirm'),
    cancelLabel: t('common.cancel'),
    destructiveIndex: 0,
    haptic: true,
  });
}
