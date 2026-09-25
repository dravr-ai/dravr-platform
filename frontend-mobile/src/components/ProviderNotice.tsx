// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The notice a provider requires before connecting (TrainingPeaks, COROS, WHOOP) and its required checkbox
// ABOUTME: One callout for the credential login and the sheet shown before an OAuth provider's authorization page

import React, { useState } from 'react';
import { Text, View } from 'react-native';
import { useTranslation } from '@pierre/i18n';
import { PROVIDER_NOTICES, type ProviderNoticeKeys } from '@pierre/shared-constants';
import { Button, Checkbox, Sheet } from './ui';

interface ProviderNoticeProps {
  notice: ProviderNoticeKeys;
  accepted: boolean;
  onAcceptedChange: (accepted: boolean) => void;
  /** Test id of the callout; its checkbox takes `${testID}-consent`. */
  testID: string;
  /** Test id of the checkbox, when it must differ from the default. */
  consentTestID?: string;
}

/**
 * The notice callout: title, body and the checkbox that states the
 * acceptance. Text on the amber tint takes the amber's bound ink.
 */
export function ProviderNotice({
  notice,
  accepted,
  onAcceptedChange,
  testID,
  consentTestID,
}: ProviderNoticeProps) {
  const { t } = useTranslation();
  return (
    <View className="mb-4 rounded-xl border border-warning/40 bg-warning/10 p-3" testID={testID}>
      <Text className="text-sm font-semibold text-on-warning-container mb-1">{t(notice.titleKey)}</Text>
      <Text className="text-sm text-on-warning-container mb-2">{t(notice.bodyKey)}</Text>
      <Checkbox
        checked={accepted}
        onChange={onAcceptedChange}
        label={t(notice.consentKey)}
        labelClassName="text-on-warning-container"
        testID={consentTestID ?? `${testID}-consent`}
      />
    </View>
  );
}

interface ProviderNoticeSheetProps {
  /** The backend whose notice is shown, or `null` while the sheet is closed. */
  provider: string | null;
  onCancel: () => void;
  /** The athlete accepted the notice; the caller starts the connect. */
  onAccept: () => void;
}

/**
 * The notice an OAuth provider (WHOOP) requires, shown before its
 * authorization page opens. Continue stays disabled until the box is ticked.
 */
export function ProviderNoticeSheet({ provider, onCancel, onAccept }: ProviderNoticeSheetProps) {
  const { t } = useTranslation();
  const notice = provider ? PROVIDER_NOTICES[provider] : undefined;
  const [accepted, setAccepted] = useState(false);

  const close = () => {
    setAccepted(false);
    onCancel();
  };

  return (
    <Sheet visible={notice !== undefined} onClose={close} testID="provider-notice-sheet">
      {notice && (
        <View>
          <ProviderNotice
            notice={notice}
            accepted={accepted}
            onAcceptedChange={setAccepted}
            testID="provider-notice"
          />
          <View className="flex-row gap-3">
            <View className="flex-1">
              <Button title={t('common.cancel')} variant="secondary" onPress={close} fullWidth testID="provider-notice-cancel" />
            </View>
            <View className="flex-1">
              <Button
                title={t('app.continue')}
                onPress={() => {
                  // Held until ticked, whatever the button's own state says.
                  if (!accepted) return;
                  setAccepted(false);
                  onAccept();
                }}
                disabled={!accepted}
                fullWidth
                testID="provider-notice-continue"
              />
            </View>
          </View>
        </View>
      )}
    </Sheet>
  );
}
