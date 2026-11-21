// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The privacy settings' health records block: upload a lab report, review what was read, the reports, the records by kind
// ABOUTME: The athlete's own session only; who else reads these records is the Sensitive health records switch beside it (owl phase 5)

import { useState } from 'react';
import { useTranslation } from '@pierre/i18n';
import { formatDate, formatDateTime } from '@pierre/chat-utils';
import {
  HEALTH_RECORDS_KEYS,
  HEALTH_RECORD_KIND_LABEL_KEY,
  describeLabUpload,
  observationName,
  observationRange,
  observationValue,
} from '@pierre/ui-logic';
import { HEALTH_RECORD_KINDS, type HealthRecordKind, type LabRecord } from '@pierre/shared-types';
import { useAuth } from '../../hooks/useAuth';
import {
  useDeleteLabDocument,
  useHealthRecordReview,
  useHealthRecords,
  useLabDocumentUpload,
  useLabDocuments,
  useReviewAnswer,
} from '../../hooks/useHealthRecords';
import { Button } from '../ui';
import { FilePickerButton } from '../ui/FilePickerButton';

/** A blood glucose value as the athlete reads it. */
const MG_DL = 'mg/dL';

/** A pending record's name, value and reference range, and the checks it failed. */
function ReviewRow({ record }: { record: LabRecord }) {
  const { t } = useTranslation();
  const answer = useReviewAnswer();
  const [failed, setFailed] = useState(false);
  const range = observationRange(record.flat);
  const decide = (decision: 'confirm' | 'reject') => {
    setFailed(false);
    answer.mutate({ id: record.id, decision }, { onError: () => setFailed(true) });
  };
  return (
    <li className="py-3 border-b ghost-border-faint last:border-b-0" data-testid={`health-review-${record.id}`}>
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <p className="text-sm text-on-surface">{observationName(record.flat)}</p>
          <p className="text-sm text-on-surface-variant" data-testid={`health-review-value-${record.id}`}>
            {observationValue(record.flat)}
            {range && <span className="ml-2">({range})</span>}
          </p>
          {record.checks.length > 0 && (
            <p className="mt-1 text-xs text-on-surface-variant" data-testid={`health-review-checks-${record.id}`}>
              {t(HEALTH_RECORDS_KEYS.checksLabel)}: {record.checks.join(', ')}
            </p>
          )}
        </div>
        <div className="flex shrink-0 gap-2">
          <Button
            variant="secondary"
            size="sm"
            onClick={() => decide('confirm')}
            disabled={answer.isPending}
            data-testid={`health-review-confirm-${record.id}`}
          >
            {t(HEALTH_RECORDS_KEYS.confirm)}
          </Button>
          <Button
            variant="tertiary"
            size="sm"
            onClick={() => decide('reject')}
            disabled={answer.isPending}
            data-testid={`health-review-reject-${record.id}`}
          >
            {t(HEALTH_RECORDS_KEYS.reject)}
          </Button>
        </div>
      </div>
      {failed && (
        <p role="alert" className="mt-2 text-sm text-error">
          {t(HEALTH_RECORDS_KEYS.failed)}
        </p>
      )}
    </li>
  );
}

/** The served records of the chosen kind. */
function RecordsOfKind({ kind }: { kind: HealthRecordKind }) {
  const { t, language } = useTranslation();
  const { data, isPending, isError } = useHealthRecords(kind);
  if (isPending) {
    return (
      <div className="flex py-3" role="status" aria-label={t('common.loading')}>
        <div className="pierre-spinner" />
      </div>
    );
  }
  if (isError) {
    return (
      <p role="alert" className="text-sm text-error">
        {t(HEALTH_RECORDS_KEYS.failed)}
      </p>
    );
  }
  const empty =
    data.records.length === 0 && data.selfReports.length === 0 && data.readings.length === 0 && data.daily.length === 0;
  if (empty) {
    return (
      <p className="text-sm text-on-surface-variant" data-testid="health-records-empty">
        {t(HEALTH_RECORDS_KEYS.recordsEmpty)}
      </p>
    );
  }
  const row = 'flex items-baseline justify-between gap-4 py-2 text-sm border-b ghost-border-faint last:border-b-0';
  return (
    <div className="space-y-4" data-testid={`health-records-${kind}`}>
      {data.records.length > 0 && (
        <ul>
          {data.records.map((record) => {
            const range = observationRange(record.flat);
            return (
              <li key={record.id} className={row} data-testid={`health-record-${record.id}`}>
                <span className="text-on-surface">{observationName(record.flat) ?? record.resourceType}</span>
                <span className="text-on-surface-variant text-right">
                  {observationValue(record.flat)}
                  {range && <span className="ml-2">({range})</span>}
                  {record.effectiveAt && <span className="ml-2">{formatDate(record.effectiveAt, language)}</span>}
                </span>
              </li>
            );
          })}
        </ul>
      )}
      {data.selfReports.length > 0 && (
        <ul>
          {data.selfReports.map((report) => (
            <li key={report.id} className={row} data-testid={`health-self-report-${report.id}`}>
              <span className="text-on-surface">{formatDate(report.start, language)}</span>
              <span className="text-on-surface-variant">
                {report.value !== null ? `${report.value}${report.unit ? ` ${report.unit}` : ''}` : report.valueCode}
              </span>
            </li>
          ))}
        </ul>
      )}
      {data.readings.length > 0 && (
        <ul>
          {data.readings.map((reading) => (
            <li key={`${reading.at}-${reading.source}`} className={row} data-testid="health-glucose-reading">
              <span className="text-on-surface">{formatDateTime(reading.at, language)}</span>
              <span className="text-on-surface-variant">
                {reading.mgDl} {MG_DL}
              </span>
            </li>
          ))}
        </ul>
      )}
      {data.daily.length > 0 && (
        <div>
          <h4 className="text-sm font-medium text-on-surface mb-1">{t(HEALTH_RECORDS_KEYS.dailyGlucose)}</h4>
          <ul>
            {data.daily.map((day) => (
              <li key={`${day.date}-${day.source}`} className={row} data-testid="health-glucose-day">
                <span className="text-on-surface">{formatDate(day.date, language, 'UTC')}</span>
                <span className="text-on-surface-variant">
                  {day.mgDl} {MG_DL}
                </span>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}

/**
 * The athlete's health records, shown to every account: a lab report's PDF
 * goes up here, each result read from it waits for the athlete's answer, and
 * the records they hold are listed by kind.
 */
export default function HealthRecordsSettings() {
  const { t, language } = useTranslation();
  const { isAuthenticated } = useAuth();
  const uploader = useLabDocumentUpload();
  const review = useHealthRecordReview(isAuthenticated);
  const documents = useLabDocuments(isAuthenticated);
  const deletion = useDeleteLabDocument();
  const [kind, setKind] = useState<HealthRecordKind>('labs');
  const [message, setMessage] = useState<{ type: 'success' | 'error'; text: string } | null>(null);
  const pending = review.data ?? [];
  const reports = documents.data ?? [];

  const remove = (id: string) => {
    setMessage(null);
    deletion.mutate(id, {
      onSuccess: () => setMessage({ type: 'success', text: t(HEALTH_RECORDS_KEYS.deleted) }),
      onError: () => setMessage({ type: 'error', text: t(HEALTH_RECORDS_KEYS.failed) }),
    });
  };

  const outcome = uploader.outcome;
  return (
    <div className="border-b ghost-border-faint pb-5 space-y-6" data-testid="health-records-settings">
      <div>
        <h3 className="text-sm font-medium text-on-surface mb-2">{t(HEALTH_RECORDS_KEYS.title)}</h3>
        <p className="text-sm text-on-surface-variant leading-relaxed mb-4">{t(HEALTH_RECORDS_KEYS.blurb)}</p>
        <FilePickerButton
          accept="application/pdf,.pdf"
          onPick={(file) => uploader.uploadFile({ size: file.size, read: () => file.arrayBuffer() })}
          disabled={uploader.isUploading}
          className="btn-base btn-secondary btn-sm focus-ring disabled:opacity-60"
          data-testid="health-records-upload"
        >
          {t(HEALTH_RECORDS_KEYS.uploadLabel)}
        </FilePickerButton>
        {uploader.isUploading && (
          <p role="status" className="mt-3 text-sm text-on-surface-variant" data-testid="health-records-uploading">
            {t(HEALTH_RECORDS_KEYS.uploading)}
          </p>
        )}
        {!uploader.isUploading && outcome !== null && (
          <p
            role={outcome.kind === 'failed' ? 'alert' : 'status'}
            className={`mt-3 text-sm ${outcome.kind === 'failed' ? 'text-error' : 'text-on-surface-variant'}`}
            data-testid={`health-records-outcome-${outcome.kind}`}
          >
            {describeLabUpload(outcome, t)}
          </p>
        )}
      </div>

      <div data-testid="health-records-review">
        <h3 className="text-sm font-medium text-on-surface mb-2">{t(HEALTH_RECORDS_KEYS.reviewTitle)}</h3>
        {pending.length === 0 ? (
          <p className="text-sm text-on-surface-variant" data-testid="health-records-review-empty">
            {t(HEALTH_RECORDS_KEYS.reviewEmpty)}
          </p>
        ) : (
          <>
            <p className="text-sm text-on-surface-variant leading-relaxed mb-2">{t(HEALTH_RECORDS_KEYS.reviewHint)}</p>
            <ul>
              {pending.map((record) => (
                <ReviewRow key={record.id} record={record} />
              ))}
            </ul>
          </>
        )}
      </div>

      <div data-testid="health-records-documents">
        <h3 className="text-sm font-medium text-on-surface mb-2">{t(HEALTH_RECORDS_KEYS.documentsTitle)}</h3>
        {reports.length === 0 ? (
          <p className="text-sm text-on-surface-variant" data-testid="health-records-documents-empty">
            {t(HEALTH_RECORDS_KEYS.documentsEmpty)}
          </p>
        ) : (
          <ul>
            {reports.map(({ document }) => (
              <li
                key={document.id}
                className="flex items-center justify-between gap-4 py-2 border-b ghost-border-faint last:border-b-0"
                data-testid={`health-document-${document.id}`}
              >
                <div className="min-w-0">
                  <p className="text-sm text-on-surface">{formatDate(document.receivedAt, language)}</p>
                  {document.status === 'no_text' && (
                    <p className="text-xs text-on-surface-variant">{t(HEALTH_RECORDS_KEYS.noText)}</p>
                  )}
                </div>
                <Button
                  variant="danger-outline"
                  size="sm"
                  onClick={() => remove(document.id)}
                  disabled={deletion.isPending}
                  data-testid={`health-document-delete-${document.id}`}
                >
                  {t(HEALTH_RECORDS_KEYS.deleteDocument)}
                </Button>
              </li>
            ))}
          </ul>
        )}
      </div>

      <div data-testid="health-records-served">
        <h3 className="text-sm font-medium text-on-surface mb-2">{t(HEALTH_RECORDS_KEYS.recordsTitle)}</h3>
        <div className="flex flex-wrap gap-2 mb-3" role="group" aria-label={t(HEALTH_RECORDS_KEYS.recordsTitle)}>
          {HEALTH_RECORD_KINDS.map((option) => (
            <button
              key={option}
              type="button"
              aria-pressed={kind === option}
              onClick={() => setKind(option)}
              className={`rounded-full px-3 py-1 text-sm focus-ring touch-target ${
                kind === option ? 'bg-primary text-on-primary' : 'border ghost-border text-on-surface-variant'
              }`}
              data-testid={`health-records-kind-${option}`}
            >
              {t(HEALTH_RECORD_KIND_LABEL_KEY[option])}
            </button>
          ))}
        </div>
        {isAuthenticated && <RecordsOfKind kind={kind} />}
      </div>

      {message && (
        <p role="status" className={`text-sm ${message.type === 'success' ? 'text-on-surface-variant' : 'text-error'}`}>
          {message.text}
        </p>
      )}
    </div>
  );
}
