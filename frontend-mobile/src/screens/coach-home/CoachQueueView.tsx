// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Board A on the phone — À traiter: today's athletes, this week's, those treated, the calm ones as one line of names
// ABOUTME: Each row is an athlete and the one reason that placed them; a tap opens the athlete, "Annuler" returns a treated one to the queue

import React from 'react';
import { Pressable, Text, View } from 'react-native';
import type { CoachRosterAthlete } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import { buildQueue, describeLine, queueCount, type QueueEntry } from '@pierre/ui-logic';
import { EmptyState, StatusDot } from '../../components/ui';
import { AthleteAvatar, GroupLabel } from './parts';
import type { Treatment } from './useTreatment';

/** A character no sentence holds, standing where the names go. */
const NAMES_SLOT = '\u0000';

interface CoachQueueViewProps {
  athletes: CoachRosterAthlete[];
  onOpen: (athleteId: string) => void;
  onSeeAll: () => void;
  undo: Treatment['undo'];
}

function QueueRow({ entry, horizon, onOpen }: { entry: QueueEntry; horizon: 'today' | 'week'; onOpen: (id: string) => void }) {
  const { t, language } = useTranslation();
  const reason = describeLine(t, language, entry.reason.key, entry.reason.params);
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`${entry.athlete.name}, ${reason}`}
      onPress={() => onOpen(entry.athlete.user_id)}
      testID={`coach-queue-row-${entry.athlete.user_id}`}
      className="min-h-[60px] flex-row items-center gap-3 px-4 py-2 active:bg-surface-container-low"
    >
      <AthleteAvatar userId={entry.athlete.user_id} name={entry.athlete.name} muted={entry.athlete.status !== 'shared'} />
      <View className="min-w-0 flex-1">
        <Text className="text-sm font-semibold text-text-primary" numberOfLines={1}>
          {entry.athlete.name}
        </Text>
        <Text className="text-sm text-text-secondary" numberOfLines={1} testID="coach-queue-reason">
          {reason}
        </Text>
      </View>
      <StatusDot tone={horizon === 'today' ? 'warning' : 'open'} testID={`coach-queue-horizon-${horizon}`} />
    </Pressable>
  );
}

/** Board A. */
export function CoachQueueView({ athletes, onOpen, onSeeAll, undo }: CoachQueueViewProps) {
  const { t, language } = useTranslation();
  const queue = React.useMemo(() => buildQueue(athletes), [athletes]);
  const total = queue.today.length + queue.week.length + queue.treated.length + queue.calm.length + queue.notSharing.length;
  const [lead, tail = ''] = t('coachHome.queue.notSharing', { names: NAMES_SLOT }).split(NAMES_SLOT);

  return (
    <View testID="coach-queue">
      <View className="flex-row items-baseline gap-2 px-4 pt-3">
        <Text accessibilityRole="header" className="font-display text-xl font-semibold text-text-primary">
          {t('coachHome.queue.title')}
        </Text>
        <Text className="font-mono text-xs text-text-secondary" testID="coach-queue-count">
          {queueCount(queue)}
        </Text>
        <Pressable
          accessibilityRole="link"
          onPress={onSeeAll}
          testID="coach-see-all"
          hitSlop={8}
          className="ml-auto flex-row items-baseline gap-1.5"
        >
          <Text className="text-sm font-medium text-primary">{t('coachHome.queue.seeAll')}</Text>
          <Text className="font-mono text-xs text-text-secondary">{total}</Text>
        </Pressable>
      </View>

      {queueCount(queue) === 0 && <EmptyState testID="coach-queue-empty">{t('coachHome.queue.empty')}</EmptyState>}

      {queue.today.length > 0 && (
        <View testID="coach-queue-today">
          <GroupLabel className="px-4 pb-1 pt-4">{t('coachHome.queue.today')}</GroupLabel>
          {queue.today.map((entry) => (
            <QueueRow key={entry.athlete.user_id} entry={entry} horizon="today" onOpen={onOpen} />
          ))}
        </View>
      )}

      {queue.week.length > 0 && (
        <View testID="coach-queue-week">
          <GroupLabel className="px-4 pb-1 pt-4">{t('coachHome.queue.week')}</GroupLabel>
          {queue.week.map((entry) => (
            <QueueRow key={entry.athlete.user_id} entry={entry} horizon="week" onOpen={onOpen} />
          ))}
        </View>
      )}

      {queue.treated.length > 0 && (
        <View testID="coach-queue-treated">
          <GroupLabel count={queue.treated.length} className="px-4 pb-1 pt-4">{t('coachHome.queue.treated')}</GroupLabel>
          {queue.treated.map((entry) => (
            <View key={entry.athlete.user_id} className="min-h-[48px] flex-row items-center gap-3 px-4 py-1.5">
              <Pressable
                accessibilityRole="button"
                onPress={() => onOpen(entry.athlete.user_id)}
                testID={`coach-treated-row-${entry.athlete.user_id}`}
                className="min-w-0 flex-1 flex-row items-center gap-3"
              >
                <AthleteAvatar userId={entry.athlete.user_id} name={entry.athlete.name} size="sm" muted />
                <Text className="min-w-0 flex-1 text-sm text-text-secondary" numberOfLines={1}>
                  <Text className="font-medium text-text-primary">{entry.athlete.name}</Text>
                  {` · ${describeLine(t, language, entry.reason.key, entry.reason.params)}`}
                </Text>
              </Pressable>
              <Pressable
                accessibilityRole="button"
                // Every treated row has an "Annuler": the name says which one this is.
                accessibilityLabel={`${t('coachHome.action.undo')}, ${entry.athlete.name}`}
                hitSlop={8}
                onPress={() => undo(entry.athlete, entry.reason)}
                testID={`coach-treated-undo-${entry.athlete.user_id}`}
              >
                <Text className="text-sm font-medium text-primary">{t('coachHome.action.undo')}</Text>
              </Pressable>
            </View>
          ))}
        </View>
      )}

      {(queue.calm.length > 0 || queue.notSharing.length > 0) && (
        <View className="mx-4 mt-5 gap-2 border-t border-border-faint pt-3" testID="coach-queue-calm">
          {queue.calm.length > 0 && (
            <Text className="text-sm text-text-secondary" testID="coach-queue-calm-line">
              <Text className="font-semibold text-text-primary">{`${t('coachHome.queue.nothing')} · ${queue.calm.length}`}</Text>
              {' — '}
              {queue.calm.map((athlete, index) => (
                <Text
                  key={athlete.user_id}
                  accessibilityRole="link"
                  onPress={() => onOpen(athlete.user_id)}
                  testID={`coach-calm-${athlete.user_id}`}
                >
                  {index > 0 ? ', ' : ''}
                  {athlete.name}
                </Text>
              ))}
            </Text>
          )}
          {queue.notSharing.length > 0 && (
            <Text className="text-xs text-text-secondary" testID="coach-queue-not-sharing">
              {lead}
              {queue.notSharing.map((athlete, index) => (
                <Text
                  key={athlete.user_id}
                  accessibilityRole="link"
                  onPress={() => onOpen(athlete.user_id)}
                  testID={`coach-not-sharing-${athlete.user_id}`}
                >
                  {index > 0 ? ', ' : ''}
                  {athlete.name}
                </Text>
              ))}
              {tail}
            </Text>
          )}
        </View>
      )}
    </View>
  );
}
