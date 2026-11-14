// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Group info's record of the human coach's reads of the member's data on the phone — who, which kind, through what, when
// ABOUTME: Shown under the member's coach sharing; the server lists the member's own reads only, newest first, kept a year

import React from 'react';
import { Text, View } from 'react-native';
import type { CoachRead } from '@pierre/shared-types';
import { formatDateTime } from '@pierre/chat-utils';
import { COACH_READ_LOG_KEYS, coachReadClassKey, coachReadSourceKey } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { useCoachReads } from '../../hooks/useGroups';

interface CoachReadLogProps {
  /** The group whose coach's reads of the member are listed. */
  groupId: string;
}

/**
 * One read: who read, which kind of data, through what, and when. A read made
 * outside a tool call — the plan view, `/plan`, the coach's roster, the
 * coach's thread — reads in plain words; a tool is named as it is.
 */
function ReadRow({ read }: { read: CoachRead }) {
  const { t, language } = useTranslation();
  const sourceKey = coachReadSourceKey(read.tool);
  return (
    <View className="py-1.5" testID="coach-read-row">
      <Text className="text-sm text-text-primary">
        <Text className="font-medium" testID="coach-read-coach">
          {read.coach_name ?? t(COACH_READ_LOG_KEYS.formerCoach)}
        </Text>
        {' · '}
        <Text testID="coach-read-class">{t(coachReadClassKey(read.data_class))}</Text>
        {' · '}
        {sourceKey ? (
          <Text className="text-text-secondary" testID="coach-read-tool">
            {t(sourceKey)}
          </Text>
        ) : (
          <Text className="font-mono text-xs text-text-tertiary" testID="coach-read-tool">
            {read.tool}
          </Text>
        )}
      </Text>
      <Text className="text-xs text-text-tertiary mt-0.5" testID="coach-read-at">
        {formatDateTime(read.read_at, language)}
      </Text>
    </View>
  );
}

/**
 * The member's own record of the group's human coach reading their data
 * through their agent (carnet#1445): each read of their training or
 * wellness, newest first, kept for the retention the server names. Only the
 * member's own reads in this group reach it.
 */
export function CoachReadLog({ groupId }: CoachReadLogProps) {
  const { t } = useTranslation();
  const { coachReads, isLoading, isError } = useCoachReads(groupId, true);

  return (
    <View className="py-2" testID="coach-read-log">
      <Text className="text-sm font-medium text-text-primary">{t(COACH_READ_LOG_KEYS.title)}</Text>
      <Text className="text-xs text-text-tertiary mt-0.5">{coachReads && t(COACH_READ_LOG_KEYS.hint, { months: coachReads.retention_months })}</Text>
      {isError ? (
        <Text className="text-xs text-text-tertiary mt-2" testID="coach-read-log-error">
          {t(COACH_READ_LOG_KEYS.loadFailed)}
        </Text>
      ) : isLoading || !coachReads ? null : coachReads.reads.length === 0 ? (
        <Text className="text-xs text-text-tertiary mt-2" testID="coach-read-log-empty">
          {t(COACH_READ_LOG_KEYS.empty, { months: coachReads.retention_months })}
        </Text>
      ) : (
        <View className="mt-1">
          {coachReads.reads.map((read) => (
            <ReadRow key={`${read.read_at}-${read.tool}-${read.coach_name ?? ''}`} read={read} />
          ))}
        </View>
      )}
    </View>
  );
}
