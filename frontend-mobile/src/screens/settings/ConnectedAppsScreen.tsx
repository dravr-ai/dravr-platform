// ABOUTME: Connected apps pane — one Section of 52 rows, the external MCP OAuth clients (e.g. Claude Desktop) approved on the consent screen
// ABOUTME: Each row names the client with its scope and grant date under it and an ink Revoke on the trailing side (Boreal v2.2 Phase 4)
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React from 'react';
import { View, Text, ActivityIndicator, Alert, RefreshControl } from 'react-native';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import type { OAuthGrant } from '@pierre/shared-types';
import { spacing, useThemeColors } from '../../constants/theme';
import { EmptyState, PaneScrollView, Row, Section } from '../../components/ui';
import { oauthApi } from '../../services/api';
import { useTranslation } from '@pierre/i18n';

const CONNECTED_APPS_QUERY_KEY = ['oauth', 'connected-apps'] as const;

function formatGrantedDate(iso: string): string {
  try {
    return new Date(iso).toLocaleDateString();
  } catch {
    return iso;
  }
}

export function ConnectedAppsScreen(): React.JSX.Element {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const queryClient = useQueryClient();

  const { data, isLoading, isError, error, refetch, isFetching } = useQuery({
    queryKey: CONNECTED_APPS_QUERY_KEY,
    queryFn: () => oauthApi.listConnectedApps(),
  });

  const revokeMutation = useMutation({
    mutationFn: (grantId: string) => oauthApi.revokeConnectedApp(grantId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: CONNECTED_APPS_QUERY_KEY });
    },
    onError: (err: unknown) => {
      const msg = err instanceof Error ? err.message : String(err);
      Alert.alert(t('app.couldNotRevokeApp'), msg);
    },
  });

  const grants = data ?? [];

  const handleRevoke = (grant: OAuthGrant): void => {
    Alert.alert(
      t('app.revokeAccessQ'),
      // The same sentence the web card's confirmation uses, from the same key:
      // revoking is the same act on both, so it cannot promise two things.
      t('app.confirmRevokeAppAccess', { app: grant.client_id }),
      [
        { text: t('common.cancel'), style: 'cancel' },
        {
          text: t('app.revoke'),
          style: 'destructive',
          onPress: () => revokeMutation.mutate(grant.id),
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
            {t('app.failedLoadConnectedApps')} {error instanceof Error ? error.message : String(error)}
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
                const revoking = revokeMutation.isPending && revokeMutation.variables === grant.id;
                return (
                  <Row
                    key={grant.id}
                    testID={`connected-app-${grant.id}`}
                    title={grant.client_id}
                    subtitle={`${grant.scope} · ${t('app.connected')} ${formatGrantedDate(grant.granted_at)}`}
                    last={index === grants.length - 1}
                    trailing={
                      revoking ? (
                        <ActivityIndicator size="small" color={colors.text.tertiary} />
                      ) : (
                        <Text
                          className="text-sm font-medium text-primary"
                          accessibilityRole="button"
                          accessibilityLabel={t('app.revokeAppLabel', { app: grant.client_id })}
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
