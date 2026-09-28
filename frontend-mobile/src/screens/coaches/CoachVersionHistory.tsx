// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The version history group of the coach editor — every stored version, a comparison with the current content, and revert
// ABOUTME: Server state through React Query; a revert asks first, refreshes the agent and its history, and hands the restored agent back

import React, { useState } from 'react';
import { View, Text, TouchableOpacity, ActivityIndicator, Alert } from 'react-native';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import { formatDateTime } from '@pierre/chat-utils';
import { useTranslation } from '@pierre/i18n';
import type { Agent, AgentFieldChange, AgentVersion } from '@pierre/shared-types';
import { coachesApi } from '../../services/api';
import { Section } from '../../components/ui';
import { useThemeColors } from '../../constants/theme';

const versionsKey = (agentId: string) => [...QUERY_KEYS.coaches.all, 'versions', agentId] as const;
const diffKey = (agentId: string, version: number) =>
  [...QUERY_KEYS.coaches.all, 'versions', agentId, 'diff', version] as const;

/** The label key for each snapshot field the server compares. */
const FIELD_LABEL_KEYS: Record<string, string> = {
  title: 'chat.agentNameLabel',
  description: 'chat.descriptionLabel',
  system_prompt: 'chat.systemPromptLabel',
  category: 'chat.categoryLabel',
  tags: 'discover.tagsSection',
  sample_prompts: 'discover.samplePrompts',
  visibility: 'discover.versionVisibilityLabel',
};

const CATEGORY_KEYS: Record<string, string> = {
  training: 'chat.categoryTraining',
  nutrition: 'chat.categoryNutrition',
  recovery: 'chat.categoryRecovery',
  recipes: 'chat.categoryRecipes',
  mobility: 'chat.categoryMobility',
  analysis: 'chat.categoryAnalysis',
  custom: 'chat.categoryCustom',
};

const VISIBILITY_KEYS: Record<string, string> = {
  private: 'discover.visibilityPrivate',
  tenant: 'discover.visibilityTenant',
  global: 'discover.visibilityGlobal',
};

type Translate = (key: string, options?: Record<string, unknown>) => string;

/** A field value as the athlete reads it: lists joined, enums named, empties said. */
function displayValue(field: string, value: unknown, t: Translate): string {
  if (value === null || value === undefined || value === '') return t('discover.versionEmptyValue');
  if (Array.isArray(value)) {
    return value.length === 0 ? t('discover.versionEmptyValue') : value.map(String).join(', ');
  }
  if (typeof value === 'string') {
    const enumKey =
      field === 'category' ? CATEGORY_KEYS[value] : field === 'visibility' ? VISIBILITY_KEYS[value] : undefined;
    return enumKey ? t(enumKey) : value;
  }
  return JSON.stringify(value);
}

function FieldChangeRow({ change }: { change: AgentFieldChange }) {
  const { t } = useTranslation();
  const labelKey = FIELD_LABEL_KEYS[change.field];
  return (
    <View className="py-2" testID={`agent-version-change-${change.field}`}>
      <Text className="text-xs font-semibold text-text-primary">{labelKey ? t(labelKey) : change.field}</Text>
      <View className="mt-1 rounded-lg bg-error/10 px-2 py-1.5">
        <Text className="text-xs font-medium text-text-secondary">{t('discover.versionThen')}</Text>
        <Text className="text-xs text-text-primary" testID="agent-version-old-value">
          {displayValue(change.field, change.old_value, t)}
        </Text>
      </View>
      <View className="mt-1 rounded-lg bg-success/10 px-2 py-1.5">
        <Text className="text-xs font-medium text-text-secondary">{t('discover.versionNow')}</Text>
        <Text className="text-xs text-text-primary" testID="agent-version-new-value">
          {displayValue(change.field, change.new_value, t)}
        </Text>
      </View>
    </View>
  );
}

function VersionDiff({ agentId, version }: { agentId: string; version: number }) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const diff = useQuery({
    queryKey: diffKey(agentId, version),
    queryFn: () => coachesApi.diffVersion(agentId, version),
  });

  if (diff.isPending) {
    return <ActivityIndicator size="small" color={colors.tokens.primary} />;
  }
  if (diff.isError) {
    return <Text className="text-xs text-error py-2">{t('discover.versionDiffLoadFailed')}</Text>;
  }
  if (diff.data.changes.length === 0) {
    return <Text className="text-xs text-text-secondary py-2">{t('discover.versionNoChanges')}</Text>;
  }
  return (
    <View testID={`agent-version-diff-${version}`}>
      {diff.data.changes.map((change) => (
        <FieldChangeRow key={change.field} change={change} />
      ))}
    </View>
  );
}

export interface CoachVersionHistoryProps {
  agentId: string;
  /** Called with the restored agent once a revert has landed. */
  onReverted: (agent: Agent) => void;
}

export function CoachVersionHistory({ agentId, onReverted }: CoachVersionHistoryProps) {
  const { t, language } = useTranslation();
  const colors = useThemeColors();
  const queryClient = useQueryClient();
  const [comparing, setComparing] = useState<number | null>(null);

  const history = useQuery({
    queryKey: versionsKey(agentId),
    queryFn: () => coachesApi.listVersions(agentId),
  });

  const revert = useMutation({
    mutationFn: (version: number) => coachesApi.revertToVersion(agentId, version),
    onSuccess: (result) => {
      setComparing(null);
      // The agent, its history and every comparison against the old current
      // content are stale now; all of them sit under the coaches prefix.
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.coaches.all });
      onReverted(result.agent);
    },
    onError: () => Alert.alert(t('common.error'), t('discover.versionRevertFailed')),
  });

  const confirmRevert = (entry: AgentVersion) => {
    Alert.alert(
      t('discover.versionRevertConfirmTitle', { version: entry.version }),
      t('discover.versionRevertConfirmMessage'),
      [
        { text: t('common.cancel'), style: 'cancel' },
        { text: t('discover.versionRevertConfirm'), onPress: () => revert.mutate(entry.version) },
      ],
    );
  };

  const renderBody = () => {
    if (history.isPending) {
      return (
        <View className="px-4">
          <ActivityIndicator size="small" color={colors.tokens.primary} />
        </View>
      );
    }
    if (history.isError) {
      return <Text className="px-4 text-sm text-error">{t('discover.versionHistoryLoadFailed')}</Text>;
    }
    if (history.data.versions.length === 0) {
      return (
        <Text className="px-4 text-sm text-text-secondary" testID="agent-version-history-empty">
          {t('discover.versionHistoryEmpty')}
        </Text>
      );
    }
    return history.data.versions.map((entry) => {
      const date = formatDateTime(entry.created_at, language);
      const isOpen = comparing === entry.version;
      return (
        <View
          key={entry.version}
          className="px-4 py-3 border-b border-border-faint"
          testID={`agent-version-${entry.version}`}
        >
          <Text className="text-sm font-medium text-text-primary">
            {t('discover.versionLabel', { version: entry.version })}
          </Text>
          <Text className="text-xs text-text-secondary" testID="agent-version-meta">
            {entry.created_by_name
              ? t('discover.versionReplacedBy', { date, author: entry.created_by_name })
              : t('discover.versionReplaced', { date })}
          </Text>
          <View className="flex-row flex-wrap gap-4 mt-2">
            <TouchableOpacity
              className="py-2"
              accessibilityRole="button"
              accessibilityState={{ expanded: isOpen }}
              onPress={() => setComparing(isOpen ? null : entry.version)}
              testID={`agent-version-compare-${entry.version}`}
            >
              <Text className="text-sm font-semibold text-primary">
                {isOpen ? t('discover.versionHideCompare') : t('discover.versionCompare')}
              </Text>
            </TouchableOpacity>
            <TouchableOpacity
              className={`py-2 ${revert.isPending ? 'opacity-50' : ''}`}
              accessibilityRole="button"
              disabled={revert.isPending}
              onPress={() => confirmRevert(entry)}
              testID={`agent-version-revert-${entry.version}`}
            >
              <Text className="text-sm font-semibold text-primary">{t('discover.versionRevert')}</Text>
            </TouchableOpacity>
          </View>
          {isOpen && <VersionDiff agentId={agentId} version={entry.version} />}
        </View>
      );
    });
  };

  return (
    <Section
      title={t('discover.versionHistoryTitle')}
      description={t('discover.versionHistoryHint')}
      className="mt-6 -mx-4"
      testID="agent-version-history"
    >
      {renderBody()}
    </Section>
  );
}
