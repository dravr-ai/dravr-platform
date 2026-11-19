// ABOUTME: "Ask a company for your data" on the web privacy pane — statutory access requests (Quebec P-39.1, GDPR) the athlete sends themselves
// ABOUTME: The fallback note to the import screen, the requests with their state and due date, the form, and one request's letter and steps

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import { useTranslation } from '@pierre/i18n';
import type { DataAccessRequest } from '@pierre/shared-types';
import { dataAccessDateLines, dataAccessStatus, formatCivilDate } from '@pierre/ui-logic';
import { useDataAccessRequests } from '../../hooks/useDataAccessRequests';
import { openImportWindow } from '../import/openImportWindow';
import { Badge, Button, Section } from '../ui';
import DataAccessRequestDetail from './DataAccessRequestDetail';
import DataAccessRequestForm from './DataAccessRequestForm';

const SHORT_DATE: Intl.DateTimeFormatOptions = { day: 'numeric', month: 'short', year: 'numeric' };

/** What the section's body shows under the list: nothing, the form (new or a draft's), or one request. */
type Pane = { kind: 'none' } | { kind: 'new' } | { kind: 'edit'; id: string } | { kind: 'view'; id: string };

function RequestRow({
  request,
  selected,
  onSelect,
}: {
  request: DataAccessRequest;
  selected: boolean;
  onSelect: () => void;
}) {
  const { t, i18n } = useTranslation();
  const status = dataAccessStatus(request);
  const dates = dataAccessDateLines(request)
    .map((line) => t(line.key, { date: formatCivilDate(line.date, i18n.language, SHORT_DATE) }))
    .join(' · ');
  return (
    <li className="border-b ghost-border-faint last:border-b-0">
      <button
        type="button"
        onClick={onSelect}
        aria-current={selected ? 'true' : undefined}
        className={`flex w-full items-center gap-3 rounded-md px-2 py-3 text-left focus:outline-none focus-visible:ring-2 focus-visible:ring-primary/50 ${
          selected ? 'bg-surface-container-low' : 'hover:bg-surface-container-low'
        }`}
        data-testid={`data-access-row-${request.id}`}
      >
        <span className="flex min-w-0 flex-1 flex-col">
          <span className="truncate text-sm font-medium text-on-surface">{request.controller}</span>
          <span className="truncate text-xs text-on-surface-variant">
            {dates || t('dataAccess.reference', { reference: request.reference })}
          </span>
        </span>
        <Badge variant={status.tone} data-testid="data-access-row-status">
          {t(status.key)}
        </Badge>
      </button>
    </li>
  );
}

export default function DataAccessRequestsSection() {
  const { t } = useTranslation();
  const { requests, isLoading, isError } = useDataAccessRequests();
  const [pane, setPane] = useState<Pane>({ kind: 'none' });

  const shown = pane.kind === 'edit' || pane.kind === 'view' ? requests?.find((r) => r.id === pane.id) : undefined;

  return (
    <Section
      title={t('dataAccess.title')}
      description={t('dataAccess.description')}
      data-testid="data-access-section"
      actions={
        pane.kind !== 'new' && (
          <Button variant="secondary" size="sm" onClick={() => setPane({ kind: 'new' })} data-testid="data-access-new">
            {t('dataAccess.newRequest')}
          </Button>
        )
      }
    >
      <div className="space-y-6">
        <div className="rounded-lg bg-surface-container-low px-4 py-3" data-testid="data-access-fallback">
          <p className="text-sm text-on-surface-variant">{t('dataAccess.fallbackNote')}</p>
          <button
            type="button"
            onClick={() => openImportWindow()}
            className="mt-2 text-sm font-medium text-primary hover:underline focus:outline-none focus-visible:ring-2 focus-visible:ring-primary/50"
            data-testid="data-access-fallback-action"
          >
            {t('dataAccess.fallbackAction')} <span aria-hidden="true">→</span>
          </button>
        </div>

        {pane.kind === 'new' && (
          <DataAccessRequestForm
            onDone={(request) => setPane({ kind: 'view', id: request.id })}
            onCancel={() => setPane({ kind: 'none' })}
          />
        )}

        <div>
          <h3 className="mb-1 text-sm font-medium text-on-surface-variant">{t('dataAccess.listTitle')}</h3>
          {isLoading && <div className="h-14 animate-pulse rounded-md bg-surface-container-low" aria-hidden="true" />}
          {isError && (
            <p className="text-sm text-error" role="alert" data-testid="data-access-load-failure">
              {t('dataAccess.error.loadFailed')}
            </p>
          )}
          {requests && requests.length === 0 && (
            <p className="py-2 text-sm text-on-surface-variant" data-testid="data-access-empty">
              {t('dataAccess.empty')}
            </p>
          )}
          {requests && requests.length > 0 && (
            <ul data-testid="data-access-list">
              {requests.map((request) => (
                <RequestRow
                  key={request.id}
                  request={request}
                  selected={shown?.id === request.id}
                  onSelect={() =>
                    setPane(shown?.id === request.id ? { kind: 'none' } : { kind: 'view', id: request.id })
                  }
                />
              ))}
            </ul>
          )}
        </div>

        {pane.kind === 'edit' && shown && (
          <DataAccessRequestForm
            key={shown.id}
            draft={shown}
            onDone={(request) => setPane({ kind: 'view', id: request.id })}
            onCancel={() => setPane({ kind: 'view', id: shown.id })}
          />
        )}
        {pane.kind === 'view' && shown && (
          <div className="rounded-lg border ghost-border-faint p-4">
            <DataAccessRequestDetail
              key={shown.id}
              request={shown}
              onEdit={() => setPane({ kind: 'edit', id: shown.id })}
            />
          </div>
        )}
      </div>
    </Section>
  );
}
