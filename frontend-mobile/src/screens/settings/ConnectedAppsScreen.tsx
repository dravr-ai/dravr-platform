// ABOUTME: Connected apps pane — the external MCP OAuth clients approved on the consent screen, the apps always allowed, and the consent history
// ABOUTME: Each app row names the client with its preset (else scope), grant date and expiry under it and an ink Revoke on the trailing side
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useMemo } from 'react';
import { View, Text, ActivityIndicator, Alert, RefreshControl } from 'react-native';
import type { OAuthGrant } from '@pierre/shared-types';
import { spacing, useThemeColors } from '../../constants/theme';
import { EmptyState, PaneScrollView, Row, Section } from '../../components/ui';
import { useConnectedApps, useRevokeConnectedApps } from '../../hooks/useAccessFeed';
import { useStandingPolicies } from '../../hooks/useGrantHistory';
import { useTranslation } from '@pierre/i18n';
import { alwaysAllowedClients, connectedAppPresetLabel, describeApiError } from '@pierre/ui-logic';
import { formatDate, formatDateTime } from '@pierre/chat-utils';
import { StandingPoliciesSection } from './StandingPoliciesSection';
import { GrantHistorySection } from './GrantHistorySection';

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
  // An always-allowed app's row says so; revoking its grant also ends the policy.
  const { data: policies } = useStandingPolicies();
  const alwaysAllowed = useMemo(() => alwaysAllowedClients(policies ?? []), [policies]);

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
                const stepUp = grant.step_up ?? null;
                const stepUpRevoking =
                  stepUp !== null && revokeMutation.isPending && (revokeMutation.variables?.includes(stepUp.id) ?? false);
                const lastGrant = index === grants.length - 1;
                return (
                  <React.Fragment key={grant.id}>
                  <Row
                    testID={`connected-app-${grant.id}`}
                    title={connectedAppName(grant)}
                    subtitle={[
                      grant.client_host,
                      connectedAppPresetLabel(grant.preset, t) ?? grant.scope,
                      alwaysAllowed.has(grant.client_id) ? t('tokens.connectedAppsAlwaysAllowed') : null,
                      t('app.connectedOn', { date: formatDate(grant.granted_at, language) }),
                      grant.expires_at && !expired
                        ? t('apiKeys.expiresOn', { date: formatDate(grant.expires_at, language) })
                        : null,
                    ]
                      .filter(Boolean)
                      .join(' · ')}
                    hint={expired ? t('providers.expired') : undefined}
                    hintTestID={`connected-app-expired-${grant.id}`}
                    last={lastGrant && stepUp === null}
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
                  {stepUp !== null && (
                    // The one-off T2 question under its app, ended on its own:
                    // the app's own access stands.
                    <Row
                      testID={`connected-app-step-up-${grant.id}`}
                      title={t('tokens.connectedAppsStepUpChild', {
                        time: stepUp.expires_at ? formatDateTime(stepUp.expires_at, language) : '',
                      })}
                      last={lastGrant}
                      trailing={
                        stepUpRevoking ? (
                          <ActivityIndicator size="small" color={colors.text.tertiary} />
                        ) : (
                          <Text
                            className="text-sm font-medium text-primary"
                            accessibilityRole="button"
                            onPress={() =>
                              revokeMutation.mutate([stepUp.id], {
                                onError: (err: unknown) => {
                                  const msg = describeApiError(err, { t, fallbackKey: 'errors.unknown' });
                                  Alert.alert(t('app.couldNotRevokeApp'), msg);
                                },
                              })
                            }
                            testID={`revoke-step-up-${grant.id}`}
                            accessibilityLabel={t('app.revokeAppLabel', { app: connectedAppName(grant) })}
                            accessibilityHint={t('tokens.connectedAppsStepUpChild', {
                              time: stepUp.expires_at ? formatDateTime(stepUp.expires_at, language) : '',
                            })}
                          >
                            {t('app.revoke')}
                          </Text>
                        )
                      }
                    />
                  )}
                  </React.Fragment>
                );
              })
            )}
          </Section>
          <StandingPoliciesSection />
          <GrantHistorySection grants={grants} policies={policies ?? []} />
        </PaneScrollView>
      )}
    </View>
  );
}
