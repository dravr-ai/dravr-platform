// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The "Bring in your history" sheet a connected card opens: sign in once to copy it, import the provider's export, or not now
// ABOUTME: Renders the card's connect-plan history choices; a choice opens its next window only once the sheet has gone

import React, { useRef } from 'react';
import { Platform, Pressable, Text, View } from 'react-native';
import { useRouter } from 'expo-router';
import { useTranslation } from '@pierre/i18n';
import { HISTORY_ACTION_KEYS, type HistoryChoice } from '@pierre/shared-constants';
import type { SciotteTarget } from '@pierre/shared-types';
import { importSourceCopy, shownHistoryChoices } from '@pierre/ui-logic';
import type { ExtendedProviderStatus } from '../types';
import { importRouteFor } from '../utils/connectFlow';
import { Sheet } from './ui';

export interface BringHistorySheetProps {
  /** The card whose history the sheet brings in; `null` while it is closed. */
  card: ExtendedProviderStatus | null;
  /** Open the export's import inside onboarding's connect step rather than over the app. */
  onboarding?: boolean;
  /** Open the one-time sign-in to `target` that copies `card`'s history. */
  onSignIn: (target: SciotteTarget, card: ExtendedProviderStatus) => void;
  onClose: () => void;
}

/**
 * iOS presents no second modal while one is on screen, so a choice closes
 * the sheet and runs once the sheet reports it has gone (`onDismiss`, which
 * only iOS sends); Android stacks them, and runs the choice at once.
 */
export function BringHistorySheet({ card, onboarding = false, onSignIn, onClose }: BringHistorySheetProps) {
  const { t } = useTranslation();
  const router = useRouter();
  // The card stays drawn while the panel slides out after `card` went null.
  const shown = useRef<ExtendedProviderStatus | null>(card);
  if (card) shown.current = card;
  const pending = useRef<(() => void) | null>(null);

  const drawn = shown.current;
  const provider = drawn?.display_name ?? '';
  const choices = drawn ? shownHistoryChoices(drawn) : [];

  const choose = (next: () => void) => {
    onClose();
    if (Platform.OS === 'ios') {
      pending.current = next;
    } else {
      next();
    }
  };
  const dismissed = () => {
    const next = pending.current;
    pending.current = null;
    next?.();
  };

  const choiceRow = (choice: HistoryChoice) => {
    if (choice.kind === 'sign_in') {
      return (
        <Pressable
          key="sign_in"
          onPress={() => drawn && choose(() => onSignIn(choice.target, drawn))}
          accessibilityRole="button"
          testID="history-choice-sign-in"
          className="py-3 border-b border-border-faint"
        >
          <Text className="text-base font-medium text-primary">{t('providers.history.choiceSignIn')}</Text>
          <Text className="text-xs text-text-tertiary mt-0.5">{t(HISTORY_ACTION_KEYS.explainer, { provider })}</Text>
        </Pressable>
      );
    }
    const copy = importSourceCopy(choice.source);
    if (!copy) return null;
    return (
      <Pressable
        key={`archive:${choice.source}`}
        onPress={() => choose(() => router.push(importRouteFor(choice.source, onboarding)))}
        accessibilityRole="button"
        testID="history-choice-archive"
        className="py-3 border-b border-border-faint"
      >
        <Text className="text-base font-medium text-primary">{t('providers.history.choiceArchive', { provider })}</Text>
        <Text className="text-xs text-text-tertiary mt-0.5">
          {copy.intake === 'link' ? t('providers.history.choiceArchiveHint', { provider }) : t(copy.hint)}
        </Text>
      </Pressable>
    );
  };

  return (
    <Sheet
      visible={card !== null}
      onClose={onClose}
      onDismiss={dismissed}
      testID="history-sheet"
      backdropTestID="history-sheet-backdrop"
    >
      <View className="px-2">
        <Text accessibilityRole="header" className="text-lg font-semibold text-text-primary">
          {t('providers.history.sheetTitle', { provider })}
        </Text>
        <Text className="text-sm text-text-secondary mt-1 mb-3">{t('providers.history.sheetBody', { provider })}</Text>
        {choices.map(choiceRow)}
        <Pressable
          onPress={onClose}
          accessibilityRole="button"
          testID="history-not-now"
          className="py-3 items-center mt-1"
        >
          <Text className="text-md font-medium text-text-secondary">{t('providers.history.notNow')}</Text>
        </Pressable>
      </View>
    </Sheet>
  );
}
