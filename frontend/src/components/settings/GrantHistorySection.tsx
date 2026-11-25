// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Connected apps' consent history — every receipt of the athlete's grants, newest first, a page at a time
// ABOUTME: Each row names the app and what happened (allowed, renewed, ended by whom, expired); a deleted app keeps its history (carnet#1544)

import { useMemo, useState } from 'react';
import type { OAuthGrant, OAuthStandingPolicy } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import {
  clientNames,
  describeApiError,
  GRANT_RECEIPT_OUTCOME_KEYS,
  GRANT_RECEIPT_PAGE,
  grantReceiptClientId,
  grantReceiptOutcome,
} from '@pierre/ui-logic';
import { formatDateTime } from '@pierre/chat-utils';
import { ACCESS_FEED_KEYS } from '@pierre/shared-constants';
import { Alert, Button, EmptyState, Section } from '../ui';
import { useGrantReceipts } from '../../hooks/useGrantHistory';

interface ReceiptPageProps {
  page: number;
  /** Whether this is the last page shown, which offers the next one when it is full. */
  last: boolean;
  names: Map<string, string>;
  onOlder: () => void;
}

/** One page of the history: its rows, and Show older under the last page when it is full. */
function ReceiptPage({ page, last, names, onOlder }: ReceiptPageProps) {
  const { t, language } = useTranslation();
  const { data: receipts, isLoading, error } = useGrantReceipts(page);

  if (isLoading) {
    return (
      <div className="flex justify-center py-6">
        <div className="pierre-spinner w-6 h-6"></div>
      </div>
    );
  }
  if (error) {
    return (
      <Alert tone="error" className="border">
        {describeApiError(error, { t, fallbackKey: 'tokens.grantHistoryLoadFailed' })}
      </Alert>
    );
  }
  const rows = receipts ?? [];
  if (page === 0 && rows.length === 0) {
    return <EmptyState>{t('tokens.grantHistoryEmpty')}</EmptyState>;
  }
  return (
    <>
      <ul>
        {rows.map((receipt) => {
          const clientId = grantReceiptClientId(receipt);
          const app = (clientId && names.get(clientId)) || clientId || t(ACCESS_FEED_KEYS.unknownApp);
          return (
            <li
              key={receipt.id}
              className="flex items-baseline justify-between gap-3 border-t ghost-border-faint py-2 first:border-t-0"
              data-testid={`grant-receipt-${receipt.id}`}
            >
              <div className="min-w-0">
                <p className="text-sm text-on-surface break-all">{app}</p>
                <p className="text-xs text-on-surface-variant">{t(GRANT_RECEIPT_OUTCOME_KEYS[grantReceiptOutcome(receipt)])}</p>
              </div>
              <time className="text-xs text-outline flex-shrink-0" dateTime={receipt.at}>
                {formatDateTime(receipt.at, language)}
              </time>
            </li>
          );
        })}
      </ul>
      {last && rows.length === GRANT_RECEIPT_PAGE && (
        <div className="pt-2">
          <Button variant="secondary" size="sm" onClick={onOlder} data-testid="grant-history-older">
            {t('tokens.grantHistoryOlder')}
          </Button>
        </div>
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
export default function GrantHistorySection({ grants, policies }: GrantHistorySectionProps) {
  const { t } = useTranslation();
  const [pages, setPages] = useState(1);
  const names = useMemo(() => clientNames(grants, policies), [grants, policies]);

  return (
    <Section title={t('tokens.grantHistory')} description={t('tokens.grantHistoryHint')} data-testid="grant-history">
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
