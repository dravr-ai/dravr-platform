// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Connected apps' consent history — every receipt of the athlete's grants, newest first, a page at a time
// ABOUTME: Each row names the app and what happened (allowed, renewed, ended by whom, expired); a deleted app keeps its history (carnet#1544)

import React, { useMemo, useState } from 'react';
import { ActivityIndicator, Text, View } from 'react-native';
import type { OAuthGrant, OAuthStandingPolicy } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import {
  clientNames,
  describeApiError,
  GRANT_RECEIPT_OUTCOME_KEYS,
  GRANT_RECEIPT_PAGE,
  grantReceiptAppName,
  grantReceiptOutcome,
} from '@pierre/ui-logic';
import { formatDateTime } from '@pierre/chat-utils';
import { ACCESS_FEED_KEYS } from '@pierre/shared-constants';
import { useThemeColors } from '../../constants/theme';
import { EmptyState, Row, Section } from '../../components/ui';
import { useGrantReceipts } from '../../hooks/useGrantHistory';

interface ReceiptPageProps {
  page: number;
  /** Whether this is the last page shown, which offers the next one when it is full. */
  last: boolean;
  names: Map<string, string>;
  onOlder: () => void;
}

/** One page of the history: its rows, and Show older under the last page when it is full. */
function ReceiptPage({ page, last, names, onOlder }: ReceiptPageProps): React.JSX.Element {
  const { t, language } = useTranslation();
  const colors = useThemeColors();
  const { data, isLoading, isError, error } = useGrantReceipts(page);

  if (isLoading) {
    return (
      <View className="items-center py-4">
        <ActivityIndicator color={colors.text.primary} />
      </View>
    );
  }
  if (isError) {
    return (
      <Text className="text-sm text-error px-4 py-3" testID={`grant-history-error-${page}`}>
        {describeApiError(error, { t, fallbackKey: 'tokens.grantHistoryLoadFailed' })}
      </Text>
    );
  }
  const receipts = data ?? [];
  if (page === 0 && receipts.length === 0) {
    return <EmptyState testID="grant-history-empty">{t('tokens.grantHistoryEmpty')}</EmptyState>;
  }
  const offersOlder = last && receipts.length === GRANT_RECEIPT_PAGE;
  return (
    <>
      {receipts.map((receipt, index) => {
        const app = grantReceiptAppName(receipt, names) || t(ACCESS_FEED_KEYS.unknownApp);
        return (
          <Row
            key={receipt.id}
            testID={`grant-receipt-${receipt.id}`}
            title={app}
            subtitle={`${t(GRANT_RECEIPT_OUTCOME_KEYS[grantReceiptOutcome(receipt)])} · ${formatDateTime(receipt.at, language)}`}
            last={last && !offersOlder && index === receipts.length - 1}
          />
        );
      })}
      {offersOlder && (
        <View className="px-4 py-3">
          <Text
            className="text-sm font-medium text-primary"
            accessibilityRole="button"
            onPress={onOlder}
            testID="grant-history-older"
          >
            {t('tokens.grantHistoryOlder')}
          </Text>
        </View>
      )}
    </>
  );
}

interface GrantHistorySectionProps {
  /** The athlete's Connected apps and standing policies, which name the apps the receipts cite. */
  grants: readonly OAuthGrant[];
  policies: readonly OAuthStandingPolicy[];
}

/** Every consent event of the athlete's apps, newest first. */
export function GrantHistorySection({ grants, policies }: GrantHistorySectionProps): React.JSX.Element {
  const { t } = useTranslation();
  const [pages, setPages] = useState(1);
  const names = useMemo(() => clientNames(grants, policies), [grants, policies]);

  return (
    <Section title={t('tokens.grantHistory')} description={t('tokens.grantHistoryHint')} testID="grant-history">
      {Array.from({ length: pages }, (_, page) => (
        <ReceiptPage
          key={page}
          page={page}
          last={page === pages - 1}
          names={names}
          onOlder={() => setPages((shown) => shown + 1)}
        />
      ))}
    </Section>
  );
}
