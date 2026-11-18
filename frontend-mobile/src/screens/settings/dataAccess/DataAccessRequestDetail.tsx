// ABOUTME: One data access request on the mobile privacy screen — its letter to copy, share as a .txt file or open in the mail app, and its deadline
// ABOUTME: The steps the athlete takes after sending it: the date sent, a GDPR extension, the answer, or closing it; Dravr never sends the letter

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useEffect, useState } from 'react';
import { ActivityIndicator, Alert, Text, View } from 'react-native';
import * as Clipboard from 'expo-clipboard';
import { File, Paths } from 'expo-file-system';
import * as Sharing from 'expo-sharing';
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
  validateAnsweredOn,
  validateExtension,
  validateSentOn,
  type DataAccessAction,
} from '@pierre/ui-logic';
import { Badge, Button } from '../../../components/ui';
import { useThemeColors } from '../../../constants/theme';
import { useDataAccessAction, useDataAccessLetter } from '../../../hooks/useDataAccessRequests';
import { openExternal } from '../../../utils/openExternal';
import { DayStepper } from './DayStepper';

const LONG_DATE: Intl.DateTimeFormatOptions = { day: 'numeric', month: 'long', year: 'numeric' };

type DatedStep = 'sent' | 'extension' | 'answered';

interface DataAccessRequestDetailProps {
  request: DataAccessRequest;
  onEdit: () => void;
}

/**
 * Write the letter to a file in the cache and open the share sheet on it,
 * where the athlete saves it to Files or hands it to another app. It leaves
 * the cache whether or not the share worked.
 */
async function shareLetter(letter: DataAccessLetter): Promise<void> {
  const file = new File(Paths.cache, letter.filename);
  try {
    if (file.exists) file.delete();
    file.create();
    file.write(letter.text);
    await Sharing.shareAsync(file.uri, { mimeType: 'text/plain', UTI: 'public.plain-text' });
  } finally {
    if (file.exists) file.delete();
  }
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
      const first = window ? (addCivilDays(window.after, 1) ?? window.after) : today;
      return {
        label: 'dataAccess.action.extendedUntil',
        hint: 'dataAccess.dates.extensionHint',
        action: 'dataAccess.action.recordExtension',
        min: first,
        max: window?.latest,
        initial: request.extended_until ?? first,
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

export function DataAccessRequestDetail({ request, onEdit }: DataAccessRequestDetailProps): React.JSX.Element {
  const { t, i18n } = useTranslation();
  const colors = useThemeColors();
  const today = dataAccessToday();
  const { letter, isLoading, errorKey } = useDataAccessLetter(request.id, request.letter_locale);
  const { run, isPending } = useDataAccessAction();
  const [copied, setCopied] = useState(false);
  const [letterError, setLetterError] = useState<string | null>(null);
  const [step, setStep] = useState<DatedStep | null>(null);
  const [day, setDay] = useState(today);
  const [stepError, setStepError] = useState<string | null>(null);

  useEffect(() => {
    if (!copied) return undefined;
    const timer = setTimeout(() => setCopied(false), 2500);
    return () => clearTimeout(timer);
  }, [copied]);

  const status = dataAccessStatus(request);
  const open = request.status === 'drafted' || request.status === 'sent';
  const field = step ? stepField(step, request, today) : null;
  const dates = dataAccessDateLines(request);

  const openStep = (next: DatedStep) => {
    setStep(next);
    setStepError(null);
    setDay(stepField(next, request, today).initial);
  };

  const settle = async (action: DataAccessAction): Promise<boolean> => {
    setStepError(null);
    const outcome = await run(action);
    if (!outcome.ok) setStepError(t(outcome.errorKey));
    return outcome.ok;
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
    setLetterError(null);
    try {
      await Clipboard.setStringAsync(letter.text);
      setCopied(true);
    } catch {
      setLetterError(t('dataAccess.error.copyFailed'));
    }
  };

  const share = async () => {
    if (!letter) return;
    setLetterError(null);
    try {
      await shareLetter(letter);
    } catch {
      setLetterError(t('dataAccess.error.letterFailed'));
    }
  };

  const confirmClose = () =>
    Alert.alert(t('dataAccess.action.close'), t('dataAccess.action.closeConfirm'), [
      { text: t('common.cancel'), style: 'cancel' },
      {
        text: t('dataAccess.action.close'),
        style: 'destructive',
        onPress: () => void settle({ kind: 'close', id: request.id }),
      },
    ]);

  return (
    <View className="gap-5 px-4" testID="data-access-detail">
      <View className="flex-row items-start justify-between gap-3">
        <View className="min-w-0 flex-1">
          <Text className="text-base font-semibold text-text-primary">{request.controller}</Text>
          <Text className="mt-0.5 text-xs text-text-secondary">
            {t('dataAccess.reference', { reference: request.reference })}
          </Text>
        </View>
        <Badge label={t(status.key)} variant={status.tone === 'secondary' ? 'neutral' : status.tone} testID="data-access-detail-status" />
      </View>

      {dates.length > 0 && (
        <View className="gap-1" testID="data-access-dates">
          {dates.map((line) => (
            <Text key={line.key} className="text-sm text-text-primary">
              {t(line.key, { date: formatCivilDate(line.date, i18n.language, LONG_DATE) })}
            </Text>
          ))}
        </View>
      )}

      {dataAccessShowsRecourse(request) && (
        <Text className="text-sm text-text-primary" accessibilityLiveRegion="polite" testID="data-access-recourse">
          {t(DATA_ACCESS_RECOURSE_KEY[request.law])}
        </Text>
      )}

      <View>
        <Text className="mb-2 text-sm font-medium text-text-secondary">{t('dataAccess.letterTitle')}</Text>
        {isLoading && <ActivityIndicator color={colors.tokens.primary} />}
        {errorKey && (
          <Text className="text-sm text-error" accessibilityRole="alert" testID="data-access-letter-failure">
            {t(errorKey)}
          </Text>
        )}
        {letter && (
          <View className="gap-3">
            <View className="rounded-lg bg-surface-container-low p-4">
              <Text selectable className="text-sm leading-5 text-text-primary" testID="data-access-letter">
                {letter.text}
              </Text>
            </View>
            <Button
              title={copied ? t('dataAccess.action.copied') : t('dataAccess.action.copy')}
              variant="secondary"
              onPress={() => void copy()}
              testID="data-access-copy"
            />
            <Button
              title={t('dataAccess.action.download')}
              variant="secondary"
              onPress={() => void share()}
              testID="data-access-download"
            />
            {letter.mailto && (
              <Button
                title={t('dataAccess.action.openMail')}
                variant="secondary"
                onPress={() => void openExternal(letter.mailto ?? '', t)}
                testID="data-access-mailto"
              />
            )}
            {letterError && (
              <Text className="text-sm text-error" accessibilityRole="alert" testID="data-access-letter-action-failure">
                {letterError}
              </Text>
            )}
          </View>
        )}
      </View>

      {open && (
        <View className="gap-3 border-t border-border-faint pt-4">
          {field && step ? (
            <View testID={`data-access-step-${step}`}>
              <DayStepper
                label={t(field.label)}
                hint={field.hint && t(field.hint)}
                value={day}
                min={field.min}
                max={field.max}
                onChange={setDay}
                testID="data-access-step-date"
              />
              <View className="gap-2">
                <Button
                  title={t(field.action)}
                  onPress={() => void submitStep()}
                  loading={isPending}
                  testID="data-access-step-confirm"
                />
                <Button title={t('common.cancel')} variant="ghost" onPress={() => setStep(null)} disabled={isPending} />
              </View>
            </View>
          ) : (
            <View className="gap-2">
              {request.status === 'drafted' && (
                <>
                  <Button title={t('dataAccess.action.markSent')} onPress={() => openStep('sent')} testID="data-access-mark-sent" />
                  <Button title={t('dataAccess.action.edit')} variant="secondary" onPress={onEdit} testID="data-access-edit" />
                </>
              )}
              {request.status === 'sent' && (
                <>
                  <Button
                    title={t('dataAccess.action.markAnswered')}
                    onPress={() => openStep('answered')}
                    testID="data-access-mark-answered"
                  />
                  {request.law === 'gdpr' && (
                    <Button
                      title={t('dataAccess.action.recordExtension')}
                      variant="secondary"
                      onPress={() => openStep('extension')}
                      testID="data-access-record-extension"
                    />
                  )}
                  <Button
                    title={t('dataAccess.action.sentOn')}
                    variant="secondary"
                    onPress={() => openStep('sent')}
                    testID="data-access-correct-sent"
                  />
                </>
              )}
              <Button
                title={t('dataAccess.action.close')}
                variant="ghost"
                onPress={confirmClose}
                testID="data-access-close"
              />
            </View>
          )}
          {stepError && (
            <Text className="text-sm text-error" accessibilityRole="alert" testID="data-access-step-failure">
              {stepError}
            </Text>
          )}
        </View>
      )}
    </View>
  );
}
