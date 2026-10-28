// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The import-your-own-data screen (M1/M2): the sources, each import's state line, and the Strava import in place
// ABOUTME: Presented as a modal from Connections and from onboarding's connect step; copy and phases are ui-logic's, shared with web

import React, { useEffect, useState } from 'react';
import { ActivityIndicator, Pressable, Text, View } from 'react-native';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from '@pierre/i18n';
import { QUERY_KEYS } from '@pierre/shared-constants';
import {
  STRAVA_ARCHIVE_SOURCE,
  isFinalImportStatus,
  type ImportLinkCheck,
} from '@pierre/shared-types';
import {
  IMPORT_POLL_INTERVAL_MS,
  classifyApiError,
  importDate,
  importDeleteCopy,
  importFailureKey,
  importHistoryCopy,
  importOutcomeKey,
  importPhase,
  importSize,
  importStateCopy,
  linkRefusalKey,
  startErrorKey,
} from '@pierre/ui-logic';
import { importsApi } from '../../services/api';
import { Button, Input, PaneScrollView, Section, Sheet } from '../../components/ui';
import { useThemeColors } from '../../constants/theme';
import { openExternal } from '../../utils/openExternal';

/** Strava's page where an athlete requests their account archive. */
const STRAVA_EXPORT_PAGE = 'https://www.strava.com/athlete/delete_your_account';

const SOURCE = STRAVA_ARCHIVE_SOURCE;

interface ImportScreenProps {
  /** Open straight on a source's import, as the Strava card's history link does. */
  initialSource?: string;
  /** Leave the screen; shown as a close action where the stack has no header (onboarding). */
  onClose?: () => void;
}

export function ImportScreen({ initialSource, onClose }: ImportScreenProps) {
  const { t, i18n } = useTranslation();
  const [source, setSource] = useState<string | null>(initialSource === SOURCE ? SOURCE : null);
  const { data: overview, isLoading } = useQuery({
    queryKey: QUERY_KEYS.imports.overview(),
    queryFn: () => importsApi.getImports(),
  });
  const stravaImport = overview?.imports.find((entry) => entry.source === SOURCE);

  return (
    <PaneScrollView testID="import-screen">
      <View className="px-4 pt-4 pb-10">
        {onClose && (
          <Pressable onPress={onClose} accessibilityRole="button" testID="import-close" className="self-end mb-2">
            <Text className="text-primary font-medium">{t('providers.import.close')}</Text>
          </Pressable>
        )}
        {source ? (
          <View>
            <Pressable onPress={() => setSource(null)} accessibilityRole="button" className="mb-3">
              <Text className="text-primary font-medium">← {t('providers.import.back')}</Text>
            </Pressable>
            <StravaImportView enabled={overview?.enabled ?? true} />
          </View>
        ) : (
          <View>
            <Text className="text-xl font-semibold text-text-primary mb-2">{t('providers.import.title')}</Text>
            <Text className="text-sm text-text-secondary mb-4">{t('providers.import.intro')}</Text>
            {isLoading && <ActivityIndicator />}
            {overview && !overview.enabled && (
              <Text className="text-sm text-text-secondary" testID="import-unavailable">
                {t('providers.import.unavailable')}
              </Text>
            )}
            {overview?.enabled && (
              <Pressable
                onPress={() => setSource(SOURCE)}
                accessibilityRole="button"
                testID="import-source-strava"
                className="flex-row items-center border-t border-b border-border-faint py-4"
              >
                <View className="flex-1">
                  <Text className="text-base font-medium text-text-primary">{t('providers.import.sources.strava')}</Text>
                  <Text className="text-xs text-text-secondary">{t('providers.import.sources.stravaHint')}</Text>
                  {stravaImport && (
                    <Text className="text-xs text-text-primary mt-1" testID="import-state-strava">
                      {(() => {
                        const line = importStateCopy(stravaImport, i18n.language);
                        return t(line.key, line.values);
                      })()}
                    </Text>
                  )}
                </View>
                <Text className="text-text-tertiary text-lg">›</Text>
              </Pressable>
            )}
          </View>
        )}
      </View>
    </PaneScrollView>
  );
}

/** The Strava import: request, link check, progress, report, history and delete. */
function StravaImportView({ enabled }: { enabled: boolean }) {
  const { t, i18n } = useTranslation();
  const colors = useThemeColors();
  const queryClient = useQueryClient();
  const [link, setLink] = useState('');
  const [check, setCheck] = useState<ImportLinkCheck | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [showReport, setShowReport] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);

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

  useEffect(() => {
    if (current && isFinalImportStatus(current.status)) {
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
    return <Text className="text-sm text-text-secondary">{t('providers.import.unavailable')}</Text>;
  }
  if (isLoading) {
    return <ActivityIndicator color={colors.tokens.primary} />;
  }
  const locale = i18n.language;
  const deleteCopy = importDeleteCopy(current?.imported ?? 0, locale);
  const historyLine = current ? importHistoryCopy(current, locale) : null;

  return (
    <View testID="strava-import">
      <Text className="text-lg font-semibold text-text-primary mb-1">{t('providers.import.sources.strava')}</Text>
      <Text className="text-sm text-text-secondary mb-4">{t('providers.import.strava.intro')}</Text>
      {notice && (
        <Text className="text-sm text-text-primary mb-3" testID="import-notice">
          {notice}
        </Text>
      )}

      {phase === 'completed' && current && (
        <Section title={t('providers.import.strava.doneTitle', { count: current.imported })} testID="import-completed">
          {current.earliest_start && current.latest_start && (
            <Text className="text-xs text-text-secondary">
              {t('providers.import.strava.doneSpan', {
                from: importDate(current.earliest_start, locale),
                to: importDate(current.latest_start, locale),
              })}
            </Text>
          )}
          {current.failed > 0 && (
            <Text className="text-sm text-text-primary mt-2">
              {t('providers.import.strava.notImported', { count: current.failed })}{' '}
              <Text className="text-primary" onPress={() => setShowReport((open) => !open)}>
                {showReport ? t('providers.import.strava.hideDetails') : t('providers.import.strava.details')}
              </Text>
            </Text>
          )}
          {showReport &&
            report.data?.issues.map((issue) => (
              <Text key={`${issue.row}-${issue.activity_id ?? ''}`} className="text-xs text-text-secondary">
                {`#${issue.row}${issue.activity_id ? ` · ${issue.activity_id}` : ''} · ${t(importOutcomeKey(issue.outcome))}`}
              </Text>
            ))}
          <Text className="text-xs font-semibold text-text-secondary mt-4">{t('providers.import.strava.historyTitle')}</Text>
          <Text className="text-sm text-text-primary">
            {historyLine && t(historyLine.key, historyLine.values)}
          </Text>
          <View className="mt-3 gap-2">
            <Button
              title={t('providers.import.strava.importNewer')}
              variant="secondary"
              onPress={() => start.mutate(undefined)}
              loading={start.isPending}
            />
            <Button
              title={t('providers.import.strava.delete')}
              variant="danger"
              onPress={() => setConfirmDelete(true)}
              testID="import-delete"
            />
          </View>
        </Section>
      )}

      {phase === 'running' && current && (
        <Section title={t('providers.import.strava.step3Title')} testID="import-running">
          <Text className="text-sm text-text-primary">
            {current.status === 'importing' && current.total_rows !== null
              ? t('providers.import.strava.progress', {
                  processed: current.processed_rows,
                  total: current.total_rows,
                })
              : t('providers.import.strava.progressStarting')}
          </Text>
          <Text className="text-xs text-text-secondary mt-2">{t('providers.import.strava.leaveHint')}</Text>
          <View className="mt-3">
            <Button
              title={t('providers.import.strava.cancel')}
              variant="secondary"
              onPress={() => cancel.mutate(current.id)}
              loading={cancel.isPending}
              testID="import-cancel"
            />
          </View>
        </Section>
      )}

      {(phase === 'request' || phase === 'awaiting_link') && (
        <View>
          {current?.status === 'failed' && (
            <Text className="text-sm text-error mb-2" testID="import-failed">
              {t(importFailureKey(current.error_code))} {t('providers.import.strava.retry')}
            </Text>
          )}
          <Section title={t('providers.import.strava.step1Title')}>
            <Text className="text-sm text-text-secondary">{t('providers.import.strava.step1Body')}</Text>
            <Text
              className="text-primary font-medium mt-3"
              onPress={() => void openExternal(STRAVA_EXPORT_PAGE, t)}
              accessibilityRole="link"
            >
              {t('providers.import.strava.step1Open')} ↗
            </Text>
            {phase === 'request' && (
              <View className="mt-3">
                <Button
                  title={t('providers.import.strava.step1Requested')}
                  variant="ghost"
                  onPress={() => start.mutate(undefined)}
                  loading={start.isPending}
                />
              </View>
            )}
          </Section>
          <Section title={t('providers.import.strava.step2Title')}>
            <Input
              label={t('providers.import.strava.linkLabel')}
              placeholder={t('providers.import.strava.linkPlaceholder')}
              value={link}
              onChangeText={(text) => {
                setLink(text);
                setCheck(null);
                setError(null);
              }}
              autoCapitalize="none"
              autoCorrect={false}
              keyboardType="url"
              testID="import-link-input"
            />
            <Text className="text-xs text-text-secondary mt-1" testID="import-link-hint">
              {t('providers.import.strava.linkHint')}
            </Text>
            {check?.accepted && (
              <Text className="text-sm text-text-primary mt-2" testID="import-link-found">
                {check.size_bytes !== null
                  ? t('providers.import.strava.found', { size: importSize(check.size_bytes) })
                  : t('providers.import.strava.foundNoSize')}
              </Text>
            )}
            <View className="mt-3">
              {check?.accepted ? (
                <Button
                  title={start.isPending ? t('providers.import.strava.starting') : t('providers.import.strava.start')}
                  onPress={() => start.mutate(link.trim())}
                  loading={start.isPending}
                  testID="import-start"
                />
              ) : (
                <Button
                  title={checkLink.isPending ? t('providers.import.strava.checking') : t('providers.import.strava.check')}
                  variant="secondary"
                  onPress={() => checkLink.mutate()}
                  disabled={!link.trim()}
                  loading={checkLink.isPending}
                  testID="import-check"
                />
              )}
            </View>
          </Section>
        </View>
      )}

      {error && (
        <Text className="text-sm text-error mt-2" testID="import-error" accessibilityRole="alert">
          {error}
        </Text>
      )}

      <Sheet visible={confirmDelete} onClose={() => setConfirmDelete(false)} testID="import-delete-sheet">
        <Text className="text-lg font-semibold text-text-primary mb-2">
          {t('providers.import.strava.deleteConfirmTitle')}
        </Text>
        <Text className="text-sm text-text-secondary mb-4">
          {t(deleteCopy.body.key, deleteCopy.body.values)}
        </Text>
        <Button
          title={t(deleteCopy.action.key, deleteCopy.action.values)}
          variant="danger"
          onPress={() => remove.mutate()}
          loading={remove.isPending}
          testID="import-delete-confirm"
        />
      </Sheet>
    </View>
  );
}
