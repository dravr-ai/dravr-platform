// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Connected apps' "Always allowed" group — each app the athlete lets renew a preset grant without the consent screen, with Stop
// ABOUTME: Stopping a policy leaves the grants it renewed; the app asks again once its access ends (carnet#1544)

import React from 'react';
import { ActivityIndicator, Alert, Text, View } from 'react-native';
import type { OAuthStandingPolicy } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import { connectedAppPresetLabel, describeApiError } from '@pierre/ui-logic';
import { formatDate } from '@pierre/chat-utils';
import { useThemeColors } from '../../constants/theme';
import { EmptyState, Row, Section } from '../../components/ui';
import { useRemoveStandingPolicy, useStandingPolicies } from '../../hooks/useGrantHistory';

/** The app's registered name, else the id it was allowed under. */
const policyAppName = (policy: OAuthStandingPolicy): string => policy.client_name || policy.client_id;

/** The apps the athlete always allows, each with a Stop that asks first. */
export function StandingPoliciesSection(): React.JSX.Element {
  const { t, language } = useTranslation();
  const colors = useThemeColors();
  const { data, isLoading, isError, error } = useStandingPolicies();
  const remove = useRemoveStandingPolicy();
  const policies = data ?? [];

  const handleStop = (policy: OAuthStandingPolicy): void => {
    Alert.alert(t('tokens.standingPolicyStopQ'), t('tokens.standingPolicyStopConfirm', { app: policyAppName(policy) }), [
      { text: t('common.cancel'), style: 'cancel' },
      {
        text: t('tokens.standingPolicyStop'),
        style: 'destructive',
        onPress: () =>
          remove.mutate(policy.id, {
            onError: (err: unknown) => {
              Alert.alert(
                t('tokens.standingPolicyStopFailed'),
                describeApiError(err, { t, fallbackKey: 'errors.unknown' }),
              );
            },
          }),
      },
    ]);
  };

  return (
    <Section
      title={t('tokens.standingPolicies')}
      description={t('tokens.standingPoliciesHint')}
      testID="standing-policies"
    >
      {isLoading ? (
        <View className="items-center py-4">
          <ActivityIndicator color={colors.text.primary} />
        </View>
      ) : isError ? (
        <Text className="text-sm text-error px-4 py-3" testID="standing-policies-error">
          {describeApiError(error, { t, fallbackKey: 'tokens.standingPoliciesLoadFailed' })}
        </Text>
      ) : policies.length === 0 ? (
        <EmptyState testID="standing-policies-empty">{t('tokens.standingPoliciesEmpty')}</EmptyState>
      ) : (
        policies.map((policy, index) => {
          const stopping = remove.isPending && remove.variables === policy.id;
          return (
            <Row
              key={policy.id}
              testID={`standing-policy-${policy.id}`}
              title={policyAppName(policy)}
              subtitle={[
                policy.client_host,
                connectedAppPresetLabel(policy.preset, t) ?? policy.preset,
                t('tokens.standingPolicySince', { date: formatDate(policy.created_at, language) }),
              ]
                .filter(Boolean)
                .join(' · ')}
              last={index === policies.length - 1}
              trailing={
                stopping ? (
                  <ActivityIndicator size="small" color={colors.text.tertiary} />
                ) : (
                  <Text
                    className="text-sm font-medium text-primary"
                    accessibilityRole="button"
                    accessibilityLabel={t('tokens.standingPolicyStopLabel', { app: policyAppName(policy) })}
                    onPress={() => handleStop(policy)}
                    testID={`stop-standing-policy-${policy.id}`}
                  >
                    {t('tokens.standingPolicyStop')}
                  </Text>
                )
              }
            />
          );
        })
      )}
    </Section>
  );
}
