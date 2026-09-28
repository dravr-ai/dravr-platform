// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The version history group of the agent edit sheet — every stored version, a comparison with the current content, and revert
// ABOUTME: Server state through React Query; a revert refreshes the agent and its history and hands the restored agent to the sheet

import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import type { Agent, AgentFieldChange, AgentVersion } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import { formatDateTime } from '@pierre/chat-utils';
import { describeApiError } from '@pierre/ui-logic';
import { coachesApi } from '../../services/api';
import { QUERY_KEYS } from '../../constants/queryKeys';
import { Button, ConfirmDialog, Section } from '../ui';

/** Cache slot for one agent's history, under the `coaches` prefix every agent mutation invalidates. */
const coachVersionsKey = (agentId: string) =>
  [...QUERY_KEYS.coaches.all, 'versions', agentId] as const;

const coachVersionDiffKey = (agentId: string, version: number) =>
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
    <li className="py-2" data-testid={`agent-version-change-${change.field}`}>
      <p className="text-xs font-semibold text-on-surface">{labelKey ? t(labelKey) : change.field}</p>
      <div className="mt-1 grid gap-2 sm:grid-cols-2">
        <div className="rounded-lg bg-error/10 px-2 py-1.5">
          <p className="text-xs font-medium text-on-surface-variant">{t('discover.versionThen')}</p>
          <p className="text-xs text-on-surface whitespace-pre-wrap break-words" data-testid="agent-version-old-value">
            {displayValue(change.field, change.old_value, t)}
          </p>
        </div>
        <div className="rounded-lg bg-success/10 px-2 py-1.5">
          <p className="text-xs font-medium text-on-surface-variant">{t('discover.versionNow')}</p>
          <p className="text-xs text-on-surface whitespace-pre-wrap break-words" data-testid="agent-version-new-value">
            {displayValue(change.field, change.new_value, t)}
          </p>
        </div>
      </div>
    </li>
  );
}

function VersionDiff({ agentId, version }: { agentId: string; version: number }) {
  const { t } = useTranslation();
  const diff = useQuery({
    queryKey: coachVersionDiffKey(agentId, version),
    queryFn: () => coachesApi.diffVersion(agentId, version),
  });

  if (diff.isPending) {
    return <div className="pierre-spinner w-4 h-4 my-2" role="status" aria-label={t('common.loading')} />;
  }
  if (diff.isError) {
    return (
      <p role="alert" className="text-xs text-error py-2">
        {describeApiError(diff.error, { t, fallbackKey: 'discover.versionDiffLoadFailed' })}
      </p>
    );
  }
  if (diff.data.changes.length === 0) {
    return <p className="text-xs text-on-surface-variant py-2">{t('discover.versionNoChanges')}</p>;
  }
  return (
    <ul className="divide-y divide-outline-variant" data-testid={`agent-version-diff-${version}`}>
      {diff.data.changes.map((change) => (
        <FieldChangeRow key={change.field} change={change} />
      ))}
    </ul>
  );
}

export interface CoachVersionHistoryProps {
  agentId: string;
  /** Called with the restored agent once a revert has landed. */
  onReverted: (agent: Agent) => void;
}

export default function CoachVersionHistory({ agentId, onReverted }: CoachVersionHistoryProps) {
  const { t, i18n } = useTranslation();
  const queryClient = useQueryClient();
  const [comparing, setComparing] = useState<number | null>(null);
  const [confirming, setConfirming] = useState<AgentVersion | null>(null);

  const history = useQuery({
    queryKey: coachVersionsKey(agentId),
    queryFn: () => coachesApi.listVersions(agentId),
  });

  const revert = useMutation({
    mutationFn: (version: number) => coachesApi.revertToVersion(agentId, version),
    onSuccess: (result) => {
      setConfirming(null);
      setComparing(null);
      // The agent, its history and every comparison against the old current
      // content are stale now; all of them sit under the coaches prefix.
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.coaches.all });
      onReverted(result.agent);
    },
    onError: () => setConfirming(null),
  });

  const renderBody = () => {
    if (history.isPending) {
      return <div className="pierre-spinner w-5 h-5" role="status" aria-label={t('common.loading')} />;
    }
    if (history.isError) {
      return (
        <p role="alert" className="text-sm text-error">
          {describeApiError(history.error, { t, fallbackKey: 'discover.versionHistoryLoadFailed' })}
        </p>
      );
    }
    if (history.data.versions.length === 0) {
      return (
        <p className="text-sm text-on-surface-variant" data-testid="agent-version-history-empty">
          {t('discover.versionHistoryEmpty')}
        </p>
      );
    }
    return (
      <ul className="divide-y divide-outline-variant">
        {history.data.versions.map((entry) => {
          const date = formatDateTime(entry.created_at, i18n.language);
          const isOpen = comparing === entry.version;
          return (
            <li key={entry.version} className="py-3" data-testid={`agent-version-${entry.version}`}>
              <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between sm:gap-3">
                <div className="min-w-0">
                  <p className="text-sm font-medium text-on-surface">
                    {t('discover.versionLabel', { version: entry.version })}
                  </p>
                  <p className="text-xs text-on-surface-variant break-words" data-testid="agent-version-meta">
                    {entry.created_by_name
                      ? t('discover.versionReplacedBy', { date, author: entry.created_by_name })
                      : t('discover.versionReplaced', { date })}
                  </p>
                </div>
                <div className="flex shrink-0 flex-wrap items-center gap-1.5">
                  <Button
                    type="button"
                    variant="secondary"
                    size="sm"
                    aria-expanded={isOpen}
                    data-testid={`agent-version-compare-${entry.version}`}
                    onClick={() => setComparing(isOpen ? null : entry.version)}
                  >
                    {isOpen ? t('discover.versionHideCompare') : t('discover.versionCompare')}
                  </Button>
                  <Button
                    type="button"
                    variant="secondary"
                    size="sm"
                    data-testid={`agent-version-revert-${entry.version}`}
                    disabled={revert.isPending}
                    onClick={() => setConfirming(entry)}
                  >
                    {t('discover.versionRevert')}
                  </Button>
                </div>
              </div>
              {isOpen && (
                <div className="mt-2">
                  <VersionDiff agentId={agentId} version={entry.version} />
                </div>
              )}
            </li>
          );
        })}
      </ul>
    );
  };

  return (
    <>
      <Section
        title={t('discover.versionHistoryTitle')}
        description={t('discover.versionHistoryHint')}
        headingLevel={3}
        data-testid="agent-version-history"
      >
        {renderBody()}
        {revert.isError && (
          <p role="alert" className="mt-2 text-xs text-error">
            {describeApiError(revert.error, { t, fallbackKey: 'discover.versionRevertFailed' })}
          </p>
        )}
      </Section>
      <ConfirmDialog
        isOpen={confirming !== null}
        onClose={() => setConfirming(null)}
        onConfirm={() => confirming && revert.mutate(confirming.version)}
        title={t('discover.versionRevertConfirmTitle', { version: confirming?.version ?? 0 })}
        message={t('discover.versionRevertConfirmMessage')}
        confirmLabel={t('discover.versionRevertConfirm')}
        cancelLabel={t('common.cancel')}
        variant="warning"
        isLoading={revert.isPending}
      />
    </>
  );
}
