// ABOUTME: Worked example shown on the connect gate (mobile) — what coaching looks like before credentials
// ABOUTME: Explicitly labelled as an example throughout; never dressed up as the viewer's own data

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useState } from 'react';
import { View, Text, Pressable } from 'react-native';
import { useTranslation } from '@pierre/i18n';

/**
 * A short illustrative exchange, collapsed by default, mirroring the web
 * `ConnectPreview`.
 *
 * The gate asks for a third-party fitness password before the user has seen
 * anything the product does. That ordering is not negotiable — the coach has
 * nothing to reason about without activity data — so the answer is to show what
 * they are being asked to unlock.
 *
 * Every line is labelled as an example and uses a named fictional athlete. It
 * must never read as the viewer's own data: someone who believes we already have
 * their numbers before connecting anything has been misled, which is worse than
 * an unpersuasive gate.
 */
export function ConnectPreview() {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);

  return (
    <View className="mt-6">
      <Pressable
        onPress={() => setOpen((v) => !v)}
        accessibilityRole="button"
        accessibilityState={{ expanded: open }}
      >
        <Text className="text-center text-sm font-medium text-text-secondary underline">
          {open ? t('app.previewHide') : t('app.previewSeeFirst')}
        </Text>
      </Pressable>

      {open && (
        <View className="mt-4 rounded-xl border border-outline-variant bg-surface-container-low p-4">
          <Text className="text-xs uppercase text-text-tertiary">{t('app.previewNotYourData')}</Text>
          <Text className="mt-1 text-xs text-text-tertiary">
            {t('app.previewAthlete')}
          </Text>

          <View className="mt-4 gap-3">
            <View className="self-end max-w-[85%] rounded-2xl border border-primary bg-primary/10 px-3.5 py-2">
              <Text className="text-sm text-on-surface">
                {t('app.previewQuestion')}
              </Text>
            </View>
            <View className="self-start max-w-[90%] rounded-2xl border border-outline-variant bg-surface-container px-3.5 py-2">
              <Text className="text-sm text-on-surface">
                Your load is up 24% on last week and you slept under six hours twice — heavy legs
                are the expected result, not a warning sign. Keep Saturday, drop it to easy pace
                and cut the last 5k.
              </Text>
            </View>
          </View>

          <Text className="mt-4 text-xs text-text-tertiary">
            {t('app.previewSpecifics')}
          </Text>
        </View>
      )}
    </View>
  );
}
