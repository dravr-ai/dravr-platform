// ABOUTME: The Strava import (W2): request the export, paste and check its link, follow the import, read its report, delete it
// ABOUTME: Polls the current import every 5 s while it runs; the copy for each state and code is ui-logic's, shared with mobile
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useEffect, useRef, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from '@pierre/i18n';
import {
  STRAVA_ARCHIVE_SOURCE,
  isFinalImportStatus,
  type ArchiveImport,
  type ImportLinkCheck,
} from '@pierre/shared-types';
import {
  IMPORT_POLL_INTERVAL_MS,
  IMPORT_RESULT_STORAGE_KEY,
  classifyApiError,
  importDate,
  importFailureKey,
  importOutcomeKey,
  importPhase,
  importSize,
  linkRefusalKey,
  startErrorKey,
} from '@pierre/ui-logic';
import { importsApi } from '../../services/api';
import { QUERY_KEYS } from '../../constants/queryKeys';
import { Button, ConfirmDialog, Input } from '../ui';

/** Strava's page where an athlete requests their account archive. */
const STRAVA_EXPORT_PAGE = 'https://www.strava.com/athlete/delete_your_account';

const SOURCE = STRAVA_ARCHIVE_SOURCE;

interface StravaImportProps {
  /** Whether imports run on this deployment. */
  enabled: boolean;
}

export default function StravaImport({ enabled }: StravaImportProps) {
  const { t, i18n } = useTranslation();
  const queryClient = useQueryClient();
  const [link, setLink] = useState('');
  const [check, setCheck] = useState<ImportLinkCheck | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showReport, setShowReport] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);

  const { data: current, isLoading } = useQuery({
    queryKey: QUERY_KEYS.imports.current(SOURCE),
    queryFn: () => importsApi.getCurrentImport(SOURCE),
    enabled,
    refetchInterval: (query) => {
      const entry = query.state.data;
      return entry && !isFinalImportStatus(entry.status) ? IMPORT_POLL_INTERVAL_MS : false;
    },
  });
  const phase = importPhase(current);

  // Tell the tab this window was opened from once an import finishes, so its
  // provider list refreshes without a reload.
  const lastStatus = useRef<ArchiveImport['status'] | null>(null);
  useEffect(() => {
    if (!current) return;
    const was = lastStatus.current;
    lastStatus.current = current.status;
    if (was && !isFinalImportStatus(was) && isFinalImportStatus(current.status)) {
      try {
        localStorage.setItem(
          IMPORT_RESULT_STORAGE_KEY,
          JSON.stringify({ source: SOURCE, status: current.status, timestamp: Date.now() }),
        );
      } catch {
        // Storage refused: the opener refreshes on its next focus instead.
      }
      void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.imports.overview() });
    }
  }, [current, queryClient]);

  const refresh = () => {
    void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.imports.all });
  };

  const checkLink = useMutation({
    mutationFn: () => importsApi.checkLink(SOURCE, link.trim()),
    onMutate: () => {
      setError(null);
      setCheck(null);
    },
    onSuccess: (result) => {
      setCheck(result);
      if (!result.accepted && result.refusal) setError(t(linkRefusalKey(result.refusal)));
    },
    onError: () => setError(t('providers.import.strava.genericError')),
  });

  const start = useMutation({
    mutationFn: (url?: string) => importsApi.startImport(SOURCE, url),
    onMutate: () => setError(null),
    onSuccess: () => {
      setLink('');
      setCheck(null);
      refresh();
    },
    onError: (err) => {
      const classified = classifyApiError(err);
      setError(t(startErrorKey(classified.status, classified.detail)));
    },
  });

  const cancel = useMutation({
    mutationFn: (importId: string) => importsApi.cancelImport(SOURCE, importId),
    onSuccess: refresh,
    onError: () => setError(t('providers.import.strava.genericError')),
  });

  const remove = useMutation({
    mutationFn: () => importsApi.deleteHistory(SOURCE),
    onSuccess: () => {
      setConfirmDelete(false);
      setNotice(t('providers.import.strava.deleted'));
      refresh();
      void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.providers.status() });
    },
    onError: () => {
      setConfirmDelete(false);
      setError(t('providers.import.strava.genericError'));
    },
  });

  const report = useQuery({
    queryKey: QUERY_KEYS.imports.report(SOURCE, current?.id ?? ''),
    queryFn: () => importsApi.getReport(SOURCE, current?.id ?? ''),
    enabled: showReport && !!current,
  });

  if (!enabled) {
    return <p className="text-sm text-on-surface-variant">{t('providers.import.unavailable')}</p>;
  }
  if (isLoading) {
    return <p className="text-sm text-on-surface-variant">{t('providers.import.loading')}</p>;
  }

  const locale = i18n.language;
  const stepClass = 'border-t ghost-border-faint py-4';

  return (
    <div data-testid="strava-import">
      <h2 className="mb-2 text-lg font-semibold text-on-surface">{t('providers.import.sources.strava')}</h2>
      <p className="mb-4 text-sm text-on-surface-variant">{t('providers.import.strava.intro')}</p>

      {notice && (
        <p role="status" className="mb-4 text-sm text-on-surface" data-testid="import-notice">
          {notice}
        </p>
      )}

      {phase === 'completed' && current && (
        <section className={stepClass} data-testid="import-completed">
          <h3 className="text-sm font-semibold text-on-surface">
            {t('providers.import.strava.doneTitle', { count: current.imported })}
          </h3>
          {current.earliest_start && current.latest_start && (
            <p className="text-xs text-on-surface-variant">
              {t('providers.import.strava.doneSpan', {
                from: importDate(current.earliest_start, locale),
                to: importDate(current.latest_start, locale),
              })}
            </p>
          )}
          {current.failed > 0 && (
            <p className="mt-2 text-sm text-on-surface">
              {t('providers.import.strava.notImported', { count: current.failed })}{' '}
              <button
                type="button"
                className="text-primary hover:text-primary-hover"
                onClick={() => setShowReport((open) => !open)}
              >
                {showReport ? t('providers.import.strava.hideDetails') : t('providers.import.strava.details')}
              </button>
            </p>
          )}
          {showReport && report.data && (
            <ul className="mt-2 space-y-1" data-testid="import-report">
              {report.data.issues.map((issue) => (
                <li key={`${issue.row}-${issue.activity_id ?? ''}`} className="text-xs text-on-surface-variant">
                  {`#${issue.row}${issue.activity_id ? ` · ${issue.activity_id}` : ''} · ${t(importOutcomeKey(issue.outcome))}`}
                </li>
              ))}
            </ul>
          )}
          <div className="mt-4">
            <h4 className="text-xs font-semibold text-on-surface-variant">
              {t('providers.import.strava.historyTitle')}
            </h4>
            <p className="text-sm text-on-surface">
              {t('providers.import.strava.historyLine', {
                count: current.imported,
                date: importDate(current.finished_at, locale),
              })}
            </p>
            <div className="mt-3 flex flex-wrap gap-2">
              <Button variant="secondary" size="sm" onClick={() => start.mutate(undefined)} loading={start.isPending}>
                {t('providers.import.strava.importNewer')}
              </Button>
              <Button variant="danger" size="sm" onClick={() => setConfirmDelete(true)}>
                {t('providers.import.strava.delete')}
              </Button>
            </div>
          </div>
        </section>
      )}

      {phase === 'running' && current && (
        <section className={stepClass} data-testid="import-running">
          <h3 className="text-sm font-semibold text-on-surface">{t('providers.import.strava.step3Title')}</h3>
          <p className="mt-1 text-sm text-on-surface" role="status">
            {current.status === 'importing' && current.total_rows !== null
              ? t('providers.import.strava.progress', {
                  processed: current.processed_rows,
                  total: current.total_rows,
                })
              : t('providers.import.strava.progressStarting')}
          </p>
          {current.status === 'importing' && current.total_rows ? (
            <div className="mt-2 h-1.5 w-full overflow-hidden rounded-full bg-surface-container-high" aria-hidden="true">
              <div
                className="h-full bg-primary transition-all"
                style={{ width: `${Math.min(100, Math.round((current.processed_rows / current.total_rows) * 100))}%` }}
              />
            </div>
          ) : null}
          <p className="mt-2 text-xs text-on-surface-variant">{t('providers.import.strava.leaveHint')}</p>
          <Button
            className="mt-3"
            variant="secondary"
            size="sm"
            onClick={() => cancel.mutate(current.id)}
            loading={cancel.isPending}
          >
            {t('providers.import.strava.cancel')}
          </Button>
        </section>
      )}

      {(phase === 'request' || phase === 'awaiting_link') && (
        <>
          {current && current.status === 'failed' && (
            <p className="mb-2 text-sm text-error" data-testid="import-failed">
              {t(importFailureKey(current.error_code))} {t('providers.import.strava.retry')}
            </p>
          )}
          <section className={stepClass}>
            <h3 className="text-sm font-semibold text-on-surface">{t('providers.import.strava.step1Title')}</h3>
            <p className="mt-1 text-sm text-on-surface-variant">{t('providers.import.strava.step1Body')}</p>
            <div className="mt-3 flex flex-wrap items-center gap-3">
              <a
                href={STRAVA_EXPORT_PAGE}
                target="_blank"
                rel="noopener noreferrer"
                className="text-sm font-medium text-primary hover:text-primary-hover"
              >
                {t('providers.import.strava.step1Open')} <span aria-hidden="true">↗</span>
              </a>
              {phase === 'request' && (
                <Button variant="tertiary" size="sm" onClick={() => start.mutate(undefined)} loading={start.isPending}>
                  {t('providers.import.strava.step1Requested')}
                </Button>
              )}
            </div>
          </section>
          <section className={stepClass}>
            <h3 className="text-sm font-semibold text-on-surface">{t('providers.import.strava.step2Title')}</h3>
            <form
              className="mt-3 space-y-3"
              onSubmit={(event) => {
                event.preventDefault();
                if (check?.accepted) {
                  start.mutate(link.trim());
                } else if (link.trim()) {
                  checkLink.mutate();
                }
              }}
            >
              <Input
                label={t('providers.import.strava.linkLabel')}
                placeholder={t('providers.import.strava.linkPlaceholder')}
                value={link}
                onChange={(event) => {
                  setLink(event.target.value);
                  setCheck(null);
                  setError(null);
                }}
                inputMode="url"
                autoComplete="off"
                spellCheck={false}
                data-testid="import-link-input"
              />
              {check?.accepted && (
                <p className="text-sm text-on-surface" data-testid="import-link-found">
                  {check.size_bytes !== null
                    ? t('providers.import.strava.found', { size: importSize(check.size_bytes) })
                    : t('providers.import.strava.foundNoSize')}
                </p>
              )}
              <div className="flex flex-wrap gap-2">
                {check?.accepted ? (
                  <Button type="submit" loading={start.isPending} data-testid="import-start">
                    {start.isPending ? t('providers.import.strava.starting') : t('providers.import.strava.start')}
                  </Button>
                ) : (
                  <Button
                    type="submit"
                    variant="secondary"
                    disabled={!link.trim()}
                    loading={checkLink.isPending}
                    data-testid="import-check"
                  >
                    {checkLink.isPending ? t('providers.import.strava.checking') : t('providers.import.strava.check')}
                  </Button>
                )}
              </div>
            </form>
          </section>
        </>
      )}

      {error && (
        <p role="alert" className="mt-2 text-sm text-error" data-testid="import-error">
          {error}
        </p>
      )}

      <ConfirmDialog
        isOpen={confirmDelete}
        onClose={() => setConfirmDelete(false)}
        onConfirm={() => remove.mutate()}
        title={t('providers.import.strava.deleteConfirmTitle')}
        message={t('providers.import.strava.deleteConfirmBody', { count: current?.imported ?? 0 })}
        confirmLabel={t('providers.import.strava.deleteConfirmAction', { count: current?.imported ?? 0 })}
        variant="danger"
        isLoading={remove.isPending}
      />
    </div>
  );
}
