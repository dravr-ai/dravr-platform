// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The health records screen, pushed from Privacy: upload a lab report, review what was read, the reports, the records by kind
// ABOUTME: The athlete's own session only; who else reads these records is Privacy's Sensitive health records switch (owl phase 5)

import React, { useState } from 'react';
import { ActivityIndicator, Alert, Pressable, Text, View } from 'react-native';
import * as DocumentPicker from 'expo-document-picker';
import { File } from 'expo-file-system';
import { useTranslation } from '@pierre/i18n';
import { formatDate, formatDateTime } from '@pierre/chat-utils';
import {
  HEALTH_RECORDS_KEYS,
  HEALTH_RECORD_KIND_LABEL_KEY,
  describeLabUpload,
  observationName,
  observationRange,
  observationValue,
  type UseLabDocumentUploadResult,
} from '@pierre/ui-logic';
import { HEALTH_RECORD_KINDS, type HealthRecordKind, type LabRecord } from '@pierre/shared-types';
import { ALERT_TONE, Button, EmptyState, PaneScrollView, Section, TextTabs } from '../../components/ui';
import { spacing, useThemeColors } from '../../constants/theme';
import { useAuth } from '../../contexts/AuthContext';
import {
  useDeleteLabDocument,
  useHealthRecordReview,
  useHealthRecords,
  useLabDocumentUpload,
  useLabDocuments,
  useReviewAnswer,
} from '../../hooks/useHealthRecords';

/** A blood glucose value as the athlete reads it. */
const MG_DL = 'mg/dL';

/** Ask the system for one PDF and upload it. */
async function pickAndUpload(uploader: UseLabDocumentUploadResult) {
  const picked = await DocumentPicker.getDocumentAsync({
    type: 'application/pdf',
    copyToCacheDirectory: true,
    multiple: false,
  });
  const asset = picked.canceled ? undefined : picked.assets?.[0];
  if (asset === undefined) return;
  uploader.uploadFile({ size: asset.size, read: () => new File(asset.uri).arrayBuffer() });
}

/** A line of the screen: a name on the leading side, its figures on the trailing side. */
function Line({ name, figure, testID }: { name: string; figure: string; testID?: string }) {
  return (
    <View className="flex-row items-baseline justify-between gap-4 px-4 py-2" testID={testID}>
      <Text className="flex-1 text-sm text-text-primary">{name}</Text>
      <Text className="text-sm text-text-secondary">{figure}</Text>
    </View>
  );
}

/** A value with its reference range, `150 g/L (135–175)`. */
function figureOf(value: string | null, range: string | null): string {
  return [value, range ? `(${range})` : null].filter(Boolean).join(' ');
}

/** A pending record's name, value and reference range, the checks it failed, and its two answers. */
function ReviewRow({ record }: { record: LabRecord }) {
  const { t } = useTranslation();
  const answer = useReviewAnswer();
  const decide = (decision: 'confirm' | 'reject') =>
    answer.mutate(
      { id: record.id, decision },
      { onError: () => Alert.alert(t('common.error'), t(HEALTH_RECORDS_KEYS.failed)) },
    );
  return (
    <View className="gap-2 px-4 py-3" testID={`health-review-${record.id}`}>
      <Text className="text-sm text-text-primary">{observationName(record.flat)}</Text>
      <Text className="text-sm text-text-secondary" testID={`health-review-value-${record.id}`}>
        {figureOf(observationValue(record.flat), observationRange(record.flat))}
      </Text>
      {record.checks.length > 0 && (
        <Text className="text-xs text-text-secondary" testID={`health-review-checks-${record.id}`}>
          {`${t(HEALTH_RECORDS_KEYS.checksLabel)}: ${record.checks.join(', ')}`}
        </Text>
      )}
      <View className="flex-row gap-2">
        <Button
          title={t(HEALTH_RECORDS_KEYS.confirm)}
          variant="secondary"
          onPress={() => decide('confirm')}
          disabled={answer.isPending}
          testID={`health-review-confirm-${record.id}`}
        />
        <Button
          title={t(HEALTH_RECORDS_KEYS.reject)}
          variant="ghost"
          onPress={() => decide('reject')}
          disabled={answer.isPending}
          testID={`health-review-reject-${record.id}`}
        />
      </View>
    </View>
  );
}

/** The served records of the chosen kind. */
function RecordsOfKind({ kind }: { kind: HealthRecordKind }) {
  const { t, language } = useTranslation();
  const colors = useThemeColors();
  const { data, isPending, isError } = useHealthRecords(kind);
  if (isPending) {
    return (
      <View className="px-4 py-3">
        <ActivityIndicator color={colors.tokens.primary} />
      </View>
    );
  }
  if (isError) {
    return <EmptyState testID="health-records-failed">{t(HEALTH_RECORDS_KEYS.failed)}</EmptyState>;
  }
  const empty =
    data.records.length === 0 && data.selfReports.length === 0 && data.readings.length === 0 && data.daily.length === 0;
  if (empty) {
    return <EmptyState testID="health-records-empty">{t(HEALTH_RECORDS_KEYS.recordsEmpty)}</EmptyState>;
  }
  return (
    <View testID={`health-records-${kind}`}>
      {data.records.map((record) => (
        <Line
          key={record.id}
          name={observationName(record.flat) ?? record.resourceType}
          figure={[
            figureOf(observationValue(record.flat), observationRange(record.flat)),
            record.effectiveAt ? formatDate(record.effectiveAt, language) : null,
          ]
            .filter(Boolean)
            .join(' · ')}
          testID={`health-record-${record.id}`}
        />
      ))}
      {data.selfReports.map((report) => (
        <Line
          key={report.id}
          name={formatDate(report.start, language)}
          figure={
            report.value !== null ? `${report.value}${report.unit ? ` ${report.unit}` : ''}` : (report.valueCode ?? '')
          }
          testID={`health-self-report-${report.id}`}
        />
      ))}
      {data.readings.map((reading) => (
        <Line
          key={`${reading.at}-${reading.source}`}
          name={formatDateTime(reading.at, language)}
          figure={`${reading.mgDl} ${MG_DL}`}
          testID="health-glucose-reading"
        />
      ))}
      {data.daily.length > 0 && (
        <View className="mt-2">
          <Text className="px-4 py-1 text-sm font-medium text-text-primary">{t(HEALTH_RECORDS_KEYS.dailyGlucose)}</Text>
          {data.daily.map((day) => (
            <Line
              key={`${day.date}-${day.source}`}
              name={formatDate(day.date, language, 'UTC')}
              figure={`${day.mgDl} ${MG_DL}`}
              testID="health-glucose-day"
            />
          ))}
        </View>
      )}
    </View>
  );
}

/**
 * The athlete's health records, as web's Privacy pane shows them: a lab
 * report's PDF goes up here, each result read from it waits for the
 * athlete's answer, and the records they hold are listed by kind.
 */
export function HealthRecordsScreen(): React.JSX.Element {
  const { t, language } = useTranslation();
  const { isAuthenticated } = useAuth();
  const uploader = useLabDocumentUpload();
  const review = useHealthRecordReview(isAuthenticated);
  const documents = useLabDocuments(isAuthenticated);
  const deletion = useDeleteLabDocument();
  const [kind, setKind] = useState<HealthRecordKind>('labs');
  const pending = review.data ?? [];
  const reports = documents.data ?? [];
  const outcome = uploader.outcome;

  const remove = (id: string) =>
    deletion.mutate(id, {
      onSuccess: () => Alert.alert(t(HEALTH_RECORDS_KEYS.deleted)),
      onError: () => Alert.alert(t('common.error'), t(HEALTH_RECORDS_KEYS.failed)),
    });

  return (
    <View className="flex-1 bg-background-primary" testID="health-records-screen">
      <PaneScrollView
        contentContainerStyle={{ paddingTop: spacing.lg, paddingBottom: spacing.xl }}
        showsVerticalScrollIndicator={false}
      >
        <View className="gap-8">
          <Section
            title={t(HEALTH_RECORDS_KEYS.title)}
            description={t(HEALTH_RECORDS_KEYS.blurb)}
            testID="health-records-section-upload"
          >
            <View className="gap-3 px-4">
              <Pressable
                onPress={() => void pickAndUpload(uploader)}
                disabled={uploader.isUploading}
                accessibilityRole="button"
                accessibilityState={{ disabled: uploader.isUploading }}
                className="min-h-11 justify-center"
                testID="health-records-upload"
              >
                <Text className={`text-sm font-medium text-primary ${uploader.isUploading ? 'opacity-60' : ''}`}>
                  {t(HEALTH_RECORDS_KEYS.uploadLabel)}
                </Text>
              </Pressable>
              {uploader.isUploading && (
                <Text
                  className="text-sm text-text-secondary"
                  accessibilityLiveRegion="polite"
                  testID="health-records-uploading"
                >
                  {t(HEALTH_RECORDS_KEYS.uploading)}
                </Text>
              )}
              {!uploader.isUploading && outcome !== null && outcome.kind !== 'failed' && (
                <Text
                  className="text-sm text-text-secondary"
                  accessibilityLiveRegion="polite"
                  testID={`health-records-outcome-${outcome.kind}`}
                >
                  {describeLabUpload(outcome, t)}
                </Text>
              )}
              {!uploader.isUploading && outcome?.kind === 'failed' && (
                <View
                  className={`rounded-lg ${ALERT_TONE.error.fill} px-3 py-2`}
                  accessibilityRole="alert"
                  accessibilityLiveRegion="polite"
                  testID="health-records-outcome-failed"
                >
                  <Text className={`text-sm ${ALERT_TONE.error.ink}`}>{describeLabUpload(outcome, t)}</Text>
                </View>
              )}
            </View>
          </Section>

          <Section
            title={t(HEALTH_RECORDS_KEYS.reviewTitle)}
            description={pending.length > 0 ? t(HEALTH_RECORDS_KEYS.reviewHint) : undefined}
            testID="health-records-review"
          >
            {pending.length === 0 ? (
              <EmptyState testID="health-records-review-empty">{t(HEALTH_RECORDS_KEYS.reviewEmpty)}</EmptyState>
            ) : (
              pending.map((record) => <ReviewRow key={record.id} record={record} />)
            )}
          </Section>

          <Section title={t(HEALTH_RECORDS_KEYS.documentsTitle)} testID="health-records-documents">
            {reports.length === 0 ? (
              <EmptyState testID="health-records-documents-empty">{t(HEALTH_RECORDS_KEYS.documentsEmpty)}</EmptyState>
            ) : (
              reports.map(({ document }) => (
                <View
                  key={document.id}
                  className="flex-row items-center justify-between gap-4 px-4 py-2"
                  testID={`health-document-${document.id}`}
                >
                  <View className="flex-1">
                    <Text className="text-sm text-text-primary">{formatDate(document.receivedAt, language)}</Text>
                    {document.status === 'no_text' && (
                      <Text className="text-xs text-text-secondary">{t(HEALTH_RECORDS_KEYS.noText)}</Text>
                    )}
                  </View>
                  <Pressable
                    onPress={() => remove(document.id)}
                    disabled={deletion.isPending}
                    accessibilityRole="button"
                    className="min-h-11 justify-center px-2"
                    testID={`health-document-delete-${document.id}`}
                  >
                    <Text className="text-sm font-medium text-error">{t(HEALTH_RECORDS_KEYS.deleteDocument)}</Text>
                  </Pressable>
                </View>
              ))
            )}
          </Section>

          <Section title={t(HEALTH_RECORDS_KEYS.recordsTitle)} testID="health-records-served">
            <TextTabs
              items={HEALTH_RECORD_KINDS.map((option) => ({
                key: option,
                label: t(HEALTH_RECORD_KIND_LABEL_KEY[option]),
              }))}
              value={kind}
              onChange={(key) => setKind(HEALTH_RECORD_KINDS.find((option) => option === key) ?? 'labs')}
              testID="health-records-kind"
              className="mb-2"
            />
            {isAuthenticated && <RecordsOfKind kind={kind} />}
          </Section>
        </View>
      </PaneScrollView>
    </View>
  );
}
