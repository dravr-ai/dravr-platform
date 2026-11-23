// ABOUTME: Connected apps pane — one Section of 52 rows, the external MCP OAuth clients (e.g. Claude Desktop) approved on the consent screen
// ABOUTME: Each row names the client with its preset (else scope), grant date and expiry under it and an ink Revoke on the trailing side (Boreal v2.2 Phase 4)
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React from 'react';
import { View, Text, ActivityIndicator, Alert, RefreshControl } from 'react-native';
import type { OAuthGrant } from '@pierre/shared-types';
import { spacing, useThemeColors } from '../../constants/theme';
import { EmptyState, PaneScrollView, Row, Section } from '../../components/ui';
import { useConnectedApps, useRevokeConnectedApps } from '../../hooks/useAccessFeed';
import { useTranslation } from '@pierre/i18n';
import { describeApiError } from '@pierre/ui-logic';
import { formatDate } from '@pierre/chat-utils';

/**
 * The name of the preset a grant was approved for, or `null` for a plain grant and for a preset
 * this build does not know yet. Each key is a literal so the catalogue gate sees it.
 */
const presetLabel = (preset: string | null, t: (key: string) => string): string | null => {
  switch (preset) {
    case 'training_coach':
      return t('tokens.connectedAppsPresetTrainingCoach');
    case 'sleep_coach':
      return t('tokens.connectedAppsPresetSleepCoach');
    default:
      return null;
  }
};

/** Whether a grant ending at `expiresAt` has already ended. */
const grantHasExpired = (expiresAt: string): boolean => Date.parse(expiresAt) <= Date.now();

/** The app's registered name, else the id it was approved under. */
const connectedAppName = (grant: OAuthGrant): string => grant.client_name || grant.client_id;

export function ConnectedAppsScreen(): React.JSX.Element {
  const { t, language } = useTranslation();
  const colors = useThemeColors();

  const { data, isLoading, isError, error, refetch, isFetching } = useConnectedApps();
  // The shared revoke rereads this list and the privacy screen's access feed alike.
  const revokeMutation = useRevokeConnectedApps();

  const grants = data ?? [];

  const handleRevoke = (grant: OAuthGrant): void => {
    Alert.alert(
      t('app.revokeAccessQ'),
      // The same sentence the web card's confirmation uses, from the same key:
      // revoking is the same act on both, so it cannot promise two things.
      t('app.confirmRevokeAppAccess', { app: connectedAppName(grant) }),
      [
        { text: t('common.cancel'), style: 'cancel' },
        {
          text: t('app.revoke'),
          style: 'destructive',
          onPress: () =>
            revokeMutation.mutate([grant.id], {
              onError: (err: unknown) => {
                const msg = describeApiError(err, { t, fallbackKey: 'errors.unknown' });
                Alert.alert(t('app.couldNotRevokeApp'), msg);
              },
            }),
        },
      ],
    );
  };

  return (
    <View className="flex-1 bg-background-primary" testID="connected-apps-screen">
      {isLoading ? (
        <View className="flex-1 items-center justify-center">
          <ActivityIndicator color={colors.text.primary} />
        </View>
      ) : isError ? (
        // The retry is a sibling, not a nested span: Android gives a nested
        // `Text` no native view, so a tap on it would reach nothing there.
        <View className="flex-row flex-wrap items-baseline px-4 py-3" testID="connected-apps-error">
          <Text className="text-sm text-error">
            {t('app.failedLoadConnectedApps')} {describeApiError(error, { t, fallbackKey: 'errors.unknown' })}
          </Text>
          <Text
            className="text-sm text-primary font-medium ml-1"
            accessibilityRole="button"
            onPress={() => {
              refetch();
            }}
            testID="connected-apps-retry"
          >
            {t('common.retry')}
          </Text>
        </View>
      ) : (
        <PaneScrollView
          contentContainerStyle={{ paddingTop: spacing.lg, paddingBottom: spacing.xl }}
          refreshControl={
            <RefreshControl
              refreshing={isFetching}
              onRefresh={() => {
                refetch();
              }}
              tintColor={colors.text.primary}
            />
          }
        >
          <Section
            title={t('tokens.connectedApps')}
            description={t('tokens.connectedAppsHint')}
            testID="connected-apps-list"
          >
            {grants.length === 0 ? (
              <EmptyState testID="connected-apps-empty">{t('tokens.connectedAppsEmpty')}</EmptyState>
            ) : (
              grants.map((grant, index) => {
                const revoking = revokeMutation.isPending && (revokeMutation.variables?.includes(grant.id) ?? false);
                const expired = grant.expires_at !== null && grantHasExpired(grant.expires_at);
                return (
                  <Row
                    key={grant.id}
                    testID={`connected-app-${grant.id}`}
                    title={connectedAppName(grant)}
                    subtitle={[
                      grant.client_host,
                      presetLabel(grant.preset, t) ?? grant.scope,
                      t('app.connectedOn', { date: formatDate(grant.granted_at, language) }),
                      grant.expires_at && !expired
                        ? t('apiKeys.expiresOn', { date: formatDate(grant.expires_at, language) })
                        : null,
                    ]
                      .filter(Boolean)
                      .join(' · ')}
                    hint={expired ? t('providers.expired') : undefined}
                    hintTestID={`connected-app-expired-${grant.id}`}
                    last={index === grants.length - 1}
                    trailing={
                      revoking ? (
                        <ActivityIndicator size="small" color={colors.text.tertiary} />
                      ) : (
                        <Text
                          className="text-sm font-medium text-primary"
                          accessibilityRole="button"
                          accessibilityLabel={t('app.revokeAppLabel', { app: connectedAppName(grant) })}
                          onPress={() => handleRevoke(grant)}
                          testID={`revoke-${grant.id}`}
                        >
                          {t('app.revoke')}
                        </Text>
                      )
                    }
                  />
                );
              })
            )}
          </Section>
        </PaneScrollView>
      )}
    </View>
  );
}
