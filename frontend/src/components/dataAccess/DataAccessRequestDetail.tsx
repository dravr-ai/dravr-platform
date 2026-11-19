// ABOUTME: One data access request on the web privacy pane — its letter to copy, download as .txt or open in the mail app, and its deadline
// ABOUTME: The steps the athlete takes after sending it: the date sent, a GDPR extension, the answer, or closing it; Dravr never sends the letter

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useEffect, useState } from 'react';
import { useTranslation } from '@pierre/i18n';
import type { DataAccessLetter, DataAccessRequest } from '@pierre/shared-types';
import {
  DATA_ACCESS_RECOURSE_KEY,
  addCivilDays,
  dataAccessDateLines,
  dataAccessShowsRecourse,
  dataAccessStatus,
  dataAccessToday,
  earliestSentOn,
  formatCivilDate,
  gdprExtensionWindow,
  isCivilDate,
  validateAnsweredOn,
  validateExtension,
  validateSentOn,
  type DataAccessAction,
} from '@pierre/ui-logic';
import { useDataAccessAction, useDataAccessLetter } from '../../hooks/useDataAccessRequests';
import { Badge, Button, ConfirmDialog, Input } from '../ui';

const LONG_DATE: Intl.DateTimeFormatOptions = { day: 'numeric', month: 'long', year: 'numeric' };

/** A step that takes a day: which one is open, and the day typed into it. */
type DatedStep = 'sent' | 'extension' | 'answered';

interface DataAccessRequestDetailProps {
  request: DataAccessRequest;
  /** Edit this draft's fields. */
  onEdit: () => void;
}

/** Hand the letter to the browser as a downloaded text file. */
function saveLetter(letter: DataAccessLetter): void {
  const url = URL.createObjectURL(new Blob([letter.text], { type: 'text/plain;charset=utf-8' }));
  const link = document.createElement('a');
  link.href = url;
  link.download = letter.filename;
  document.body.appendChild(link);
  link.click();
  link.remove();
  URL.revokeObjectURL(url);
}

/** The bounds and first value of a dated step's field, and the sentence a value is refused with. */
function stepField(step: DatedStep, request: DataAccessRequest, today: string) {
  switch (step) {
    case 'sent':
      return {
        label: 'dataAccess.action.sentOn',
        hint: 'dataAccess.dates.countsFrom',
        action: 'dataAccess.action.markSent',
        min: earliestSentOn(today),
        max: today,
        initial: request.sent_on ?? today,
        check: (date: string) => validateSentOn(date, today),
      };
    case 'extension': {
      const window = gdprExtensionWindow(request.sent_on);
      return {
        label: 'dataAccess.action.extendedUntil',
        hint: 'dataAccess.dates.extensionHint',
        action: 'dataAccess.action.recordExtension',
        min: window ? (addCivilDays(window.after, 1) ?? window.after) : undefined,
        max: window?.latest,
        initial: request.extended_until ?? (window ? (addCivilDays(window.after, 1) ?? '') : ''),
        check: (date: string) => validateExtension(date, request),
      };
    }
    case 'answered':
      return {
        label: 'dataAccess.action.answeredOn',
        hint: undefined,
        action: 'dataAccess.action.markAnswered',
        min: request.sent_on ?? undefined,
        max: today,
        initial: today,
        check: (date: string) => validateAnsweredOn(date, request.sent_on),
      };
  }
}

export default function DataAccessRequestDetail({ request, onEdit }: DataAccessRequestDetailProps) {
  const { t, i18n } = useTranslation();
  const today = dataAccessToday();
  const { letter, isLoading, errorKey } = useDataAccessLetter(request.id, request.letter_locale);
  const { run, isPending } = useDataAccessAction();
  const [copied, setCopied] = useState(false);
  const [copyFailed, setCopyFailed] = useState(false);
  const [step, setStep] = useState<DatedStep | null>(null);
  const [day, setDay] = useState('');
  const [stepError, setStepError] = useState<string | null>(null);
  const [confirmClose, setConfirmClose] = useState(false);

  useEffect(() => {
    if (!copied) return undefined;
    const timer = setTimeout(() => setCopied(false), 2500);
    return () => clearTimeout(timer);
  }, [copied]);

  const status = dataAccessStatus(request);
  const open = request.status === 'drafted' || request.status === 'sent';
  const field = step ? stepField(step, request, today) : null;

  const openStep = (next: DatedStep) => {
    setStep(next);
    setStepError(null);
    setDay(stepField(next, request, today).initial);
  };

  const settle = async (action: DataAccessAction): Promise<boolean> => {
    setStepError(null);
    const outcome = await run(action);
    if (!outcome.ok) {
      setStepError(t(outcome.errorKey));
      return false;
    }
    return true;
  };

  const submitStep = async () => {
    if (!step || !field) return;
    const refused = field.check(day);
    if (refused) {
      setStepError(t(refused));
      return;
    }
    if (await settle({ kind: step, id: request.id, date: day })) setStep(null);
  };

  const copy = async () => {
    if (!letter) return;
    setCopyFailed(false);
    try {
      await navigator.clipboard.writeText(letter.text);
      setCopied(true);
    } catch {
      setCopyFailed(true);
    }
  };

  return (
    <div className="space-y-5" data-testid="data-access-detail">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <h3 className="text-sm font-semibold text-on-surface">{request.controller}</h3>
          <p className="mt-0.5 text-xs text-on-surface-variant">
            {t('dataAccess.reference', { reference: request.reference })}
          </p>
        </div>
        <Badge variant={status.tone} data-testid="data-access-detail-status">
          {t(status.key)}
        </Badge>
      </div>

      {dataAccessDateLines(request).length > 0 && (
        <ul className="space-y-1 text-sm text-on-surface" data-testid="data-access-dates">
          {dataAccessDateLines(request).map((line) => (
            <li key={line.key}>{t(line.key, { date: formatCivilDate(line.date, i18n.language, LONG_DATE) })}</li>
          ))}
        </ul>
      )}

      {dataAccessShowsRecourse(request) && (
        <p className="text-sm text-on-surface" role="status" data-testid="data-access-recourse">
          {t(DATA_ACCESS_RECOURSE_KEY[request.law])}
        </p>
      )}

      <div>
        <h4 className="mb-2 text-sm font-medium text-on-surface-variant">{t('dataAccess.letterTitle')}</h4>
        {isLoading && <div className="h-40 animate-pulse rounded-lg bg-surface-container-low" aria-hidden="true" />}
        {errorKey && (
          <p className="text-sm text-error" role="alert" data-testid="data-access-letter-failure">
            {t(errorKey)}
          </p>
        )}
        {letter && (
          <>
            <pre
              className="max-h-96 overflow-y-auto whitespace-pre-wrap break-words rounded-lg border ghost-border-faint bg-surface-container-low p-4 font-sans text-sm leading-relaxed text-on-surface"
              lang={letter.locale}
              data-testid="data-access-letter"
            >
              {letter.text}
            </pre>
            <div className="mt-3 flex flex-wrap gap-2">
              <Button variant="secondary" size="sm" onClick={copy} data-testid="data-access-copy">
                {copied ? t('dataAccess.action.copied') : t('dataAccess.action.copy')}
              </Button>
              <Button variant="secondary" size="sm" onClick={() => saveLetter(letter)} data-testid="data-access-download">
                {t('dataAccess.action.download')}
              </Button>
              {letter.mailto && (
                <a className="btn-base btn-secondary btn-sm" href={letter.mailto} data-testid="data-access-mailto">
                  {t('dataAccess.action.openMail')}
                </a>
              )}
            </div>
            {copyFailed && (
              <p className="mt-2 text-sm text-error" role="alert" data-testid="data-access-copy-failure">
                {t('dataAccess.error.copyFailed')}
              </p>
            )}
            <span className="sr-only" role="status">
              {copied ? t('dataAccess.action.copied') : ''}
            </span>
          </>
        )}
      </div>

      {open && (
        <div className="border-t ghost-border-faint pt-4">
          {field && step ? (
            <div className="space-y-3" data-testid={`data-access-step-${step}`}>
              <div className="max-w-xs">
                <Input
                  type="date"
                  label={t(field.label)}
                  helpText={field.hint && t(field.hint)}
                  min={field.min}
                  max={field.max}
                  value={day}
                  onChange={(event) => setDay(event.target.value)}
                  data-testid="data-access-step-date"
                />
              </div>
              <div className="flex flex-wrap gap-2">
                <Button
                  size="sm"
                  onClick={submitStep}
                  loading={isPending}
                  disabled={!isCivilDate(day)}
                  data-testid="data-access-step-confirm"
                >
                  {t(field.action)}
                </Button>
                <Button variant="tertiary" size="sm" onClick={() => setStep(null)} disabled={isPending}>
                  {t('common.cancel')}
                </Button>
              </div>
            </div>
          ) : (
            <div className="flex flex-wrap gap-2">
              {request.status === 'drafted' && (
                <>
                  <Button size="sm" onClick={() => openStep('sent')} data-testid="data-access-mark-sent">
                    {t('dataAccess.action.markSent')}
                  </Button>
                  <Button variant="secondary" size="sm" onClick={onEdit} data-testid="data-access-edit">
                    {t('dataAccess.action.edit')}
                  </Button>
                </>
              )}
              {request.status === 'sent' && (
                <>
                  <Button size="sm" onClick={() => openStep('answered')} data-testid="data-access-mark-answered">
                    {t('dataAccess.action.markAnswered')}
                  </Button>
                  {request.law === 'gdpr' && (
                    <Button
                      variant="secondary"
                      size="sm"
                      onClick={() => openStep('extension')}
                      data-testid="data-access-record-extension"
                    >
                      {t('dataAccess.action.recordExtension')}
                    </Button>
                  )}
                  <Button variant="secondary" size="sm" onClick={() => openStep('sent')} data-testid="data-access-correct-sent">
                    {t('dataAccess.action.sentOn')}
                  </Button>
                </>
              )}
              <Button
                variant="danger-outline"
                size="sm"
                onClick={() => setConfirmClose(true)}
                data-testid="data-access-close"
              >
                {t('dataAccess.action.close')}
              </Button>
            </div>
          )}
          {stepError && (
            <p className="mt-3 text-sm text-error" role="alert" data-testid="data-access-step-failure">
              {stepError}
            </p>
          )}
        </div>
      )}

      <ConfirmDialog
        isOpen={confirmClose}
        onClose={() => setConfirmClose(false)}
        onConfirm={async () => {
          // A refusal is said under the steps, so the dialog closes either way.
          await settle({ kind: 'close', id: request.id });
          setConfirmClose(false);
        }}
        title={t('dataAccess.action.close')}
        message={t('dataAccess.action.closeConfirm')}
        confirmLabel={t('dataAccess.action.close')}
        variant="warning"
        isLoading={isPending}
      />
    </div>
  );
}
