// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Presents a connected provider's actions as the platform's own menu — an action sheet on iOS, a dialog on Android
// ABOUTME: Reconnect when the token lapsed, withdraw AI use when given, disconnect as the destructive row, with haptics

import type { TFunction } from '@pierre/i18n';
import { AI_CONSENT_KEYS } from '@pierre/shared-constants';
import { presentMenu, type MenuRow } from '../../utils/presentMenu';

interface PresentProviderMenuOptions {
  /** The provider's display name; the sheet's title on both platforms. */
  providerName: string;
  /** Whether the connection needs a fresh authorization, the only case that offers a reconnect. */
  canReconnect: boolean;
  onReconnect: () => void;
  onDisconnect: () => void;
  /**
   * The destructive row's word, when it is not "Disconnect": a connection
   * read through the athlete's coach ends that link, so it reads "Unlink".
   */
  disconnectLabel?: string;
  /**
   * Withdraws the consent to AI use of the provider's data, when it is given
   * (carnet#726): one row, and the connection stays.
   */
  onWithdrawAiConsent?: () => void;
}

/**
 * Show a provider's menu and run whichever row the athlete picks.
 *
 * The rows are the two things a connection row used to draw as its own
 * buttons; a press on the row presents them through the system's menu
 * instead, so the list carries the provider, its state and nothing else.
 */
export function presentProviderMenu(
  {
    providerName,
    canReconnect,
    onReconnect,
    onDisconnect,
    disconnectLabel,
    onWithdrawAiConsent,
  }: PresentProviderMenuOptions,
  t: TFunction,
): void {
  const rows: MenuRow[] = [];
  if (canReconnect) {
    rows.push({ label: t('app.reconnect'), onPress: onReconnect });
  }
  if (onWithdrawAiConsent) {
    rows.push({ label: t(AI_CONSENT_KEYS.withdraw), onPress: onWithdrawAiConsent });
  }
  rows.push({ label: disconnectLabel ?? t('app.disconnect'), onPress: onDisconnect });
  // Disconnect is always the last row, and the one destructive row.
  presentMenu(rows, {
    title: providerName,
    titleOnIos: true,
    cancelLabel: t('common.cancel'),
    destructiveIndex: rows.length - 1,
    haptic: true,
  });
}
