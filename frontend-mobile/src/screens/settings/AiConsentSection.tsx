// ABOUTME: The privacy screen's consent switches: AI use per connected provider that asks, both sensitive tiers, and workouts sent to a provider
// ABOUTME: Withdrawing is one tap and stops it at once; the data stays visible and the connection stays live (carnet#726, carnet#1390)
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React from 'react';
import { Alert, Switch, Text, View } from 'react-native';
import { useQuery } from '@tanstack/react-query';
import {
  AI_CONSENT_KEYS,
  QUERY_KEYS,
  SENSITIVE_CONSENT_KEYS,
  SENSITIVE_T2_CONSENT,
  SENSITIVE_T3_CONSENT,
  TRANSFER_CONSENT_KEYS,
  aiConsentCards,
  transferConsentCards,
} from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { Section, switchColors } from '../../components/ui';
import { useThemeColors } from '../../constants/theme';
import { oauthApi } from '../../services/api';
import { useAuth } from '../../contexts/AuthContext';
import { useAiConsent, useAiConsentState } from '../../hooks/useAiConsent';
import { useTransferConsent } from '../../hooks/useTransferConsent';

/**
 * One switch per connected provider whose consent to AI use the athlete can
 * give or withdraw, shown while one is connected, the Sensitive health
 * records switch, shown to every account, and one switch per connected
 * provider Dravr may send planned workouts to (Garmin), shown while one is
 * connected. Each switch shows the server's answer, reread after every
 * change, never the tap alone.
 */
export function AiConsentSection(): React.JSX.Element {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const { isAuthenticated } = useAuth();
  const { data } = useQuery({
    queryKey: QUERY_KEYS.providers.status(),
    queryFn: () => oauthApi.getProvidersStatus(),
    enabled: isAuthenticated,
  });
  const consent = useAiConsent();
  const sensitive = useAiConsentState(SENSITIVE_T2_CONSENT, isAuthenticated);
  const intimate = useAiConsentState(SENSITIVE_T3_CONSENT, isAuthenticated);
  const transfer = useTransferConsent();
  const cards = aiConsentCards(data?.providers ?? []);
  const transferCards = transferConsentCards(data?.providers ?? []);
  const change = (provider: string, allow: boolean) =>
    consent.mutate(
      { provider, allow },
      { onError: () => Alert.alert(t('common.error'), t(AI_CONSENT_KEYS.failed)) },
    );
  const changeTransfer = (provider: string, allow: boolean) =>
    transfer.mutate(
      { provider, allow },
      { onError: () => Alert.alert(t('common.error'), t(TRANSFER_CONSENT_KEYS.failed)) },
    );
  const classes = [
    { key: SENSITIVE_T2_CONSENT, label: t(SENSITIVE_CONSENT_KEYS.t2Label), state: sensitive.data },
    {
      key: SENSITIVE_T3_CONSENT,
      label: t(SENSITIVE_CONSENT_KEYS.t3Label),
      hint: t(SENSITIVE_CONSENT_KEYS.t3Hint),
      state: intimate.data,
    },
  ];

  return (
    <>
      {cards.length > 0 && (
        <Section title={t(AI_CONSENT_KEYS.title)} description={t(AI_CONSENT_KEYS.blurb)} testID="privacy-section-ai-consent">
          <View className="gap-3 px-4">
            {cards.map((card) => {
              const label = t(AI_CONSENT_KEYS.label, { provider: card.display_name });
              return (
                <View key={card.provider} className="flex-row items-center justify-between gap-4">
                  <Text className="flex-1 text-sm text-text-primary">{label}</Text>
                  <Switch
                    testID={`ai-consent-switch-${card.provider}`}
                    accessibilityLabel={label}
                    value={card.ai_consent === true}
                    onValueChange={(allow) => change(card.provider, allow)}
                    {...switchColors(colors.tokens, card.ai_consent === true)}
                    disabled={consent.isPending}
                  />
                </View>
              );
            })}
          </View>
        </Section>
      )}
      <Section
        title={t(SENSITIVE_CONSENT_KEYS.title)}
        description={t(SENSITIVE_CONSENT_KEYS.blurb)}
        testID="privacy-section-sensitive-records"
      >
        <View className="gap-3 px-4">
          {classes.map(({ key, label, hint, state }) => (
            <View key={key} className="flex-row items-center justify-between gap-4">
              <View className="flex-1">
                <Text className="text-sm text-text-primary">{label}</Text>
                {hint && (
                  <Text className="mt-1 text-xs text-text-secondary" testID={`ai-consent-hint-${key}`}>
                    {hint}
                  </Text>
                )}
              </View>
              <Switch
                testID={`ai-consent-switch-${key}`}
                accessibilityLabel={label}
                value={state === true}
                onValueChange={(allow) => change(key, allow)}
                {...switchColors(colors.tokens, state === true)}
                disabled={consent.isPending || state === undefined}
              />
            </View>
          ))}
        </View>
      </Section>
      {transferCards.length > 0 && (
        <Section
          title={t(TRANSFER_CONSENT_KEYS.title)}
          description={t(TRANSFER_CONSENT_KEYS.blurb)}
          testID="privacy-section-transfer-consent"
        >
          <View className="gap-3 px-4">
            {transferCards.map((card) => {
              const label = t(TRANSFER_CONSENT_KEYS.label, { provider: card.display_name });
              return (
                <View key={card.provider} className="flex-row items-center justify-between gap-4">
                  <Text className="flex-1 text-sm text-text-primary">{label}</Text>
                  <Switch
                    testID={`transfer-consent-switch-${card.provider}`}
                    accessibilityLabel={label}
                    value={card.transfer_consent === true}
                    onValueChange={(allow) => changeTransfer(card.provider, allow)}
                    {...switchColors(colors.tokens, card.transfer_consent === true)}
                    disabled={transfer.isPending}
                  />
                </View>
              );
            })}
          </View>
        </Section>
      )}
    </>
  );
}
