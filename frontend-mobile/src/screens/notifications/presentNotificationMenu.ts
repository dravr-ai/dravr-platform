// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Presents a notification's actions as the platform's own menu — an action sheet on iOS, a dialog on Android
// ABOUTME: Delete is the one row and it is destructive; selection haptics fire as the menu opens

import type { TFunction } from '@pierre/i18n';
import { presentMenu } from '../../utils/presentMenu';

interface PresentNotificationMenuOptions {
  onDelete: () => void;
}

/**
 * Show a notification's menu and run whichever row is picked.
 *
 * Reading a notification, and any action it carries, already happens from the
 * row's own tap, which opens the detail modal — so the only thing this menu
 * offers is deleting it: one destructive row and cancel, mirroring
 * `presentMemoryFactMenu`'s shape exactly (Boreal v2.2 Phase 5, P5.8).
 */
export function presentNotificationMenu({ onDelete }: PresentNotificationMenuOptions, t: TFunction): void {
  presentMenu([{ label: t('common.delete'), onPress: onDelete }], {
    title: t('shell.notificationDelete'),
    cancelLabel: t('common.cancel'),
    destructiveIndex: 0,
    haptic: true,
  });
}
