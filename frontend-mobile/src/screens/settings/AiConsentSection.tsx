// ABOUTME: The privacy screen's consent-to-AI-use switches, one per connected provider whose card carries one
// ABOUTME: Withdrawing is one tap and stops AI use at once; the data stays visible and the connection stays live (carnet#726)
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React from 'react';
import { Alert, Switch, Text, View } from 'react-native';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { AI_CONSENT_KEYS, QUERY_KEYS, aiConsentCards } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { Section } from '../../components/ui';
import { useThemeColors } from '../../constants/theme';
import { oauthApi } from '../../services/api';
import { useAuth } from '../../contexts/AuthContext';

/** One consent change: `allow` gives it, `!allow` withdraws it. */
interface AiConsentChange {
  provider: string;
  allow: boolean;
}

/**
 * One switch per connected provider whose consent to AI use the athlete can
 * give or withdraw; nothing while no such provider is connected. Each switch
 * shows the server's answer, reread after every change, never the tap alone.
 */
export function AiConsentSection(): React.JSX.Element | null {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const { isAuthenticated } = useAuth();
  const queryClient = useQueryClient();
  const { data } = useQuery({
    queryKey: QUERY_KEYS.user.providerConnections(),
    queryFn: () => oauthApi.getProvidersStatus(),
    enabled: isAuthenticated,
  });
  const consent = useMutation({
    mutationFn: ({ provider, allow }: AiConsentChange) =>
      allow ? oauthApi.grantAiConsent(provider) : oauthApi.withdrawAiConsent(provider),
    onError: () => Alert.alert(t('common.error'), t(AI_CONSENT_KEYS.failed)),
    onSettled: () => queryClient.invalidateQueries({ queryKey: QUERY_KEYS.user.providerConnections() }),
  });
  const cards = aiConsentCards(data?.providers ?? []);

  if (cards.length === 0) {
    return null;
  }

  return (
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
                onValueChange={(allow) => consent.mutate({ provider: card.provider, allow })}
                trackColor={{ false: colors.border.default, true: colors.tokens.primary }}
                disabled={consent.isPending}
              />
            </View>
          );
        })}
      </View>
    </Section>
  );
}
