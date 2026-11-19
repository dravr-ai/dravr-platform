// ABOUTME: "Ask a company for your data" on the mobile privacy screen — statutory access requests (Quebec P-39.1, GDPR) the athlete sends themselves
// ABOUTME: The fallback note to the import screen, the requests with their state and due date, the form, and one request's letter and steps

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useState } from 'react';
import { ActivityIndicator, Pressable, Text, View } from 'react-native';
import { useRouter } from 'expo-router';
import { useTranslation } from '@pierre/i18n';
import { dataAccessDateLines, dataAccessStatus, formatCivilDate } from '@pierre/ui-logic';
import { Badge, Row, Section } from '../../../components/ui';
import { useThemeColors } from '../../../constants/theme';
import { useDataAccessRequests } from '../../../hooks/useDataAccessRequests';
import { IMPORT_ROUTE } from '../../../navigation/routes';
import { DataAccessRequestDetail } from './DataAccessRequestDetail';
import { DataAccessRequestForm } from './DataAccessRequestForm';

const SHORT_DATE: Intl.DateTimeFormatOptions = { day: 'numeric', month: 'short', year: 'numeric' };

type Pane = { kind: 'none' } | { kind: 'new' } | { kind: 'edit'; id: string } | { kind: 'view'; id: string };

export function DataAccessRequestsSection(): React.JSX.Element {
  const { t, i18n } = useTranslation();
  const colors = useThemeColors();
  const router = useRouter();
  const { requests, isLoading, isError } = useDataAccessRequests();
  const [pane, setPane] = useState<Pane>({ kind: 'none' });

  const shown = pane.kind === 'edit' || pane.kind === 'view' ? requests?.find((r) => r.id === pane.id) : undefined;

  return (
    <Section
      title={t('dataAccess.title')}
      description={t('dataAccess.description')}
      testID="privacy-section-data-access"
      actions={
        pane.kind !== 'new' ? (
          <Text
            className="text-sm font-medium text-primary"
            accessibilityRole="button"
            onPress={() => setPane({ kind: 'new' })}
            testID="data-access-new"
          >
            {t('dataAccess.newRequest')}
          </Text>
        ) : undefined
      }
    >
      <View className="gap-6">
        <View className="mx-4 rounded-lg bg-surface-container-low px-4 py-3" testID="data-access-fallback">
          <Text className="text-sm text-text-secondary">{t('dataAccess.fallbackNote')}</Text>
          <Pressable
            onPress={() => router.push(IMPORT_ROUTE)}
            accessibilityRole="link"
            className="mt-2 min-h-11 justify-center"
            testID="data-access-fallback-action"
          >
            <Text className="text-sm font-medium text-primary">{t('dataAccess.fallbackAction')} →</Text>
          </Pressable>
        </View>

        {pane.kind === 'new' && (
          <DataAccessRequestForm
            onDone={(request) => setPane({ kind: 'view', id: request.id })}
            onCancel={() => setPane({ kind: 'none' })}
          />
        )}

        <View>
          <Text className="mb-1 px-4 text-sm font-medium text-text-secondary">{t('dataAccess.listTitle')}</Text>
          {isLoading && <ActivityIndicator color={colors.tokens.primary} />}
          {isError && (
            <Text className="px-4 text-sm text-error" accessibilityRole="alert" testID="data-access-load-failure">
              {t('dataAccess.error.loadFailed')}
            </Text>
          )}
          {requests && requests.length === 0 && (
            <Text className="px-4 py-2 text-sm text-text-secondary" testID="data-access-empty">
              {t('dataAccess.empty')}
            </Text>
          )}
          {requests?.map((request, index) => {
            const status = dataAccessStatus(request);
            const dates = dataAccessDateLines(request)
              .map((line) => t(line.key, { date: formatCivilDate(line.date, i18n.language, SHORT_DATE) }))
              .join(' · ');
            return (
              <Row
                key={request.id}
                title={request.controller}
                subtitle={dates || t('dataAccess.reference', { reference: request.reference })}
                trailing={
                  <Badge label={t(status.key)} variant={status.tone === 'secondary' ? 'neutral' : status.tone} />
                }
                showChevron={false}
                last={index === requests.length - 1}
                accessibilityState={{ selected: shown?.id === request.id }}
                onPress={() =>
                  setPane(shown?.id === request.id ? { kind: 'none' } : { kind: 'view', id: request.id })
                }
                testID={`data-access-row-${request.id}`}
              />
            );
          })}
        </View>

        {pane.kind === 'edit' && shown && (
          <DataAccessRequestForm
            key={shown.id}
            draft={shown}
            onDone={(request) => setPane({ kind: 'view', id: request.id })}
            onCancel={() => setPane({ kind: 'view', id: shown.id })}
          />
        )}
        {pane.kind === 'view' && shown && (
          <DataAccessRequestDetail
            key={shown.id}
            request={shown}
            onEdit={() => setPane({ kind: 'edit', id: shown.id })}
          />
        )}
      </View>
    </Section>
  );
}
