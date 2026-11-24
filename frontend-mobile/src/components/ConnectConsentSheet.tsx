// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The sheet a connect shows before it starts when it offers consents it does not require (Garmin's AI use and workout transfer)
// ABOUTME: One unticked box per consent with its hint and policy link; Continue is never held and hands over the ticked keys

import React, { useEffect, useState } from 'react';
import { Text, View } from 'react-native';
import { useTranslation } from '@pierre/i18n';
import { GARMIN_CONSENT_KEYS, type ConnectConsent } from '@pierre/shared-constants';
import { Button, Checkbox, Sheet } from './ui';
import { openExternal } from '../utils/openExternal';

export interface ConnectConsentSheetProps {
  visible: boolean;
  /** The boxes the connect offers (`connectConsentsFor`), in the order shown. */
  consents: readonly ConnectConsent[];
  onCancel: () => void;
  /** The athlete continues: the keys they ticked, none included. */
  onContinue: (keys: string[]) => void;
}

/**
 * Every box starts unticked, on every opening, and the connect goes through
 * with none ticked (carnet#1157, carnet#1390): Continue is never held.
 */
export function ConnectConsentSheet({ visible, consents, onCancel, onContinue }: ConnectConsentSheetProps) {
  const { t } = useTranslation();
  const [ticked, setTicked] = useState<ReadonlySet<string>>(new Set());

  useEffect(() => {
    if (visible) setTicked(new Set());
  }, [visible]);

  const toggle = (key: string, checked: boolean) =>
    setTicked((current) => {
      const next = new Set(current);
      if (checked) next.add(key);
      else next.delete(key);
      return next;
    });

  return (
    <Sheet visible={visible} onClose={onCancel} testID="connect-consent-sheet">
      <View>
        <Text className="text-lg font-semibold text-text-primary mb-1">{t(GARMIN_CONSENT_KEYS.title)}</Text>
        <Text className="text-sm text-text-secondary mb-4">{t(GARMIN_CONSENT_KEYS.intro)}</Text>
        {consents.map((consent) => {
          const { linkUrl, linkKey } = consent;
          return (
            <View key={consent.key} className="mb-3" testID={`connect-consent-${consent.key}`}>
              <Checkbox
                checked={ticked.has(consent.key)}
                onChange={(checked) => toggle(consent.key, checked)}
                label={t(consent.labelKey)}
                testID={`connect-consent-box-${consent.key}`}
              />
              <Text className="ml-7 text-xs text-text-secondary">{t(consent.hintKey)}</Text>
              {linkUrl && linkKey ? (
                <Text
                  className="ml-7 mt-1 text-xs font-medium text-primary self-start"
                  onPress={() => void openExternal(linkUrl, t)}
                  accessibilityRole="link"
                  testID={`connect-consent-link-${consent.key}`}
                >
                  {t(linkKey)}
                </Text>
              ) : null}
            </View>
          );
        })}
        <View className="flex-row gap-3 mt-2">
          <View className="flex-1">
            <Button title={t('common.cancel')} variant="secondary" onPress={onCancel} fullWidth testID="connect-consent-cancel" />
          </View>
          <View className="flex-1">
            <Button
              title={t(GARMIN_CONSENT_KEYS.continue)}
              onPress={() => onContinue(consents.map((consent) => consent.key).filter((key) => ticked.has(key)))}
              fullWidth
              testID="connect-consent-continue"
            />
          </View>
        </View>
      </View>
    </Sheet>
  );
}
