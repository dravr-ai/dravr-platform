// ABOUTME: The privacy screen's consent-to-AI-use switches: one per connected provider whose card carries one, and health records
// ABOUTME: Withdrawing is one tap and stops AI use at once; the data stays visible and the connection stays live (carnet#726, J2)
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
  aiConsentCards,
} from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { Section, switchColors } from '../../components/ui';
import { useThemeColors } from '../../constants/theme';
import { oauthApi } from '../../services/api';
import { useAuth } from '../../contexts/AuthContext';
import { useAiConsent, useAiConsentState } from '../../hooks/useAiConsent';

/**
 * One switch per connected provider whose consent to AI use the athlete can
 * give or withdraw, shown while one is connected, and the Sensitive health
 * records switch, shown to every account. Each switch shows the server's
 * answer, reread after every change, never the tap alone.
 */
export function AiConsentSection(): React.JSX.Element {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const { isAuthenticated } = useAuth();
  const { data } = useQuery({
    queryKey: QUERY_KEYS.user.providerConnections(),
    queryFn: () => oauthApi.getProvidersStatus(),
    enabled: isAuthenticated,
  });
  const consent = useAiConsent();
  const sensitive = useAiConsentState(SENSITIVE_T2_CONSENT, isAuthenticated);
  const cards = aiConsentCards(data?.providers ?? []);
  const change = (provider: string, allow: boolean) =>
    consent.mutate(
      { provider, allow },
      { onError: () => Alert.alert(t('common.error'), t(AI_CONSENT_KEYS.failed)) },
    );
  const sensitiveLabel = t(SENSITIVE_CONSENT_KEYS.t2Label);
  const recordsAllowed = sensitive.data === true;

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
        <View className="flex-row items-center justify-between gap-4 px-4">
          <Text className="flex-1 text-sm text-text-primary">{sensitiveLabel}</Text>
          <Switch
            testID={`ai-consent-switch-${SENSITIVE_T2_CONSENT}`}
            accessibilityLabel={sensitiveLabel}
            value={recordsAllowed}
            onValueChange={(allow) => change(SENSITIVE_T2_CONSENT, allow)}
            {...switchColors(colors.tokens, recordsAllowed)}
            disabled={consent.isPending || sensitive.data === undefined}
          />
        </View>
      </Section>
    </>
  );
}
