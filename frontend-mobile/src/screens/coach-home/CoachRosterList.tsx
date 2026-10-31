// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Board B on the phone — Mes athlètes as a two-line list, sorted by days without activity, never by form, with the non-sharers apart
// ABOUTME: Form is each athlete's own band on a six-step strip; a non-sharer shows a status line and nothing of their training

import React, { useMemo, useState } from 'react';
import { Pressable, Text, View } from 'react-native';
import type { CoachRosterAthlete, CoachRosterGroup } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import {
  athletesInGroup,
  buildRosterTable,
  daysToARace,
  formWords,
  lastSessionWords,
  lastWhenWords,
  nonSharerStatusWords,
} from '@pierre/ui-logic';
import { EmptyState, StatusDot, TextTabs } from '../../components/ui';
import { AthleteAvatar, FormStrip, GroupLabel } from './parts';
import type { CoachDestinations } from './useCoachDestinations';

interface CoachRosterListProps {
  athletes: CoachRosterAthlete[];
  groups: CoachRosterGroup[];
  today: string;
  onOpen: (athleteId: string) => void;
  destinations: CoachDestinations;
}

const ALL = 'all';

function SharerRow({ athlete, today, onOpen }: { athlete: CoachRosterAthlete; today: string; onOpen: (id: string) => void }) {
  const { t, language } = useTranslation();
  const last = lastWhenWords(t, athlete.training?.days_since_last_activity);
  const form = formWords(t, language, athlete);
  const session = lastSessionWords(t, athlete);
  const toRace = daysToARace(athlete, today);
  // No hover title on a phone: a cut reading shows its date in the line itself.
  const line = `${form.asOf ?? form.text} · ${last.text}`;
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`${athlete.name}, ${line}`}
      onPress={() => onOpen(athlete.user_id)}
      testID={`coach-row-${athlete.user_id}`}
      className="min-h-[60px] flex-row items-center gap-3 border-b border-border-faint py-2 active:bg-surface-container-low"
    >
      <AthleteAvatar userId={athlete.user_id} name={athlete.name} size="sm" />
      <View className="min-w-0 flex-1 gap-0.5">
        <View className="flex-row items-center gap-2">
          <Text className="text-sm font-semibold text-text-primary" numberOfLines={1}>
            {athlete.name}
          </Text>
          {toRace !== null && (
            <Text className="ml-auto font-mono text-xs text-text-secondary" testID={`coach-row-race-${athlete.user_id}`}>
              {t('coachHome.detail.countdown', { days: toRace })}
            </Text>
          )}
        </View>
        <View className="min-w-0 flex-row items-center gap-2">
          <FormStrip band={athlete.training?.form?.band ?? 'insufficient_history'} muted={form.cut} />
          <Text
            className="min-w-0 flex-1 text-xs text-text-secondary"
            numberOfLines={1}
            testID={`coach-row-line-${athlete.user_id}`}
          >
            {session === '' ? line : `${line} · ${session}`}
          </Text>
        </View>
      </View>
    </Pressable>
  );
}

/** Board B. */
export function CoachRosterList({ athletes, groups, today, onOpen, destinations }: CoachRosterListProps) {
  const { t, language } = useTranslation();
  const [groupId, setGroupId] = useState<string>(ALL);
  const inGroup = useMemo(() => athletesInGroup(athletes, groupId === ALL ? null : groupId), [athletes, groupId]);
  const table = useMemo(() => buildRosterTable(inGroup), [inGroup]);
  const tabs = useMemo(
    () => [
      { key: ALL, label: `${t('coachHome.roster.allGroups')} ${athletes.length}` },
      ...groups.map((group) => ({ key: group.id, label: `${group.name} ${group.athlete_count}` })),
    ],
    [athletes.length, groups, t],
  );
  const empty = table.sharers.length === 0 && table.nonSharers.length === 0;

  return (
    <View testID="coach-roster">
      <View className="flex-row items-baseline gap-2 px-4 pt-3">
        <Text accessibilityRole="header" className="font-display text-xl font-semibold text-text-primary">
          {t('coachHome.roster.title')}
        </Text>
        <Text className="font-mono text-xs text-text-secondary" testID="coach-roster-count">
          {inGroup.length}
        </Text>
      </View>
      {groups.length > 1 && <TextTabs items={tabs} value={groupId} onChange={setGroupId} testID="coach-group-tabs" className="mt-2" />}
      <Text className="px-4 pb-1 pt-2 text-xs text-text-secondary" testID="coach-roster-sort">
        {t('coachHome.roster.sort')}
      </Text>

      {empty && <EmptyState testID="coach-roster-empty">{t('coachHome.noAthletes')}</EmptyState>}

      {table.sharers.length > 0 && (
        <View className="px-4" testID="coach-roster-list">
          {table.sharers.map((athlete) => (
            <SharerRow key={athlete.user_id} athlete={athlete} today={today} onOpen={onOpen} />
          ))}
        </View>
      )}

      {table.nonSharers.length > 0 && (
        <View className="px-4" testID="coach-roster-non-sharers">
          <GroupLabel count={table.nonSharers.length} className="pb-1 pt-5">
            {t('coachHome.roster.noShareTitle')}
          </GroupLabel>
          {table.nonSharers.map((athlete) => (
            <View
              key={athlete.user_id}
              testID={`coach-nonsharer-${athlete.user_id}`}
              className="min-h-[60px] flex-row items-center gap-3 border-b border-border-faint py-2"
            >
              <Pressable
                accessibilityRole="button"
                onPress={() => onOpen(athlete.user_id)}
                testID={`coach-nonsharer-open-${athlete.user_id}`}
                className="min-w-0 flex-1 flex-row items-center gap-3"
              >
                <AthleteAvatar userId={athlete.user_id} name={athlete.name} size="sm" muted />
                <View className="min-w-0 flex-1">
                  <Text className="text-sm font-semibold text-text-primary" numberOfLines={1}>
                    {athlete.name}
                  </Text>
                  <View className="flex-row items-center gap-2">
                    <StatusDot tone="neutral" />
                    <Text className="min-w-0 flex-1 text-xs text-text-secondary">{nonSharerStatusWords(t, language, athlete)}</Text>
                  </View>
                </View>
              </Pressable>
              {athlete.groups.length > 0 && (
                <Pressable
                  accessibilityRole="link"
                  // Every non-sharer row has this link: the name says whose group it opens.
                  accessibilityLabel={`${t('coachHome.action.writeToAthlete')}, ${athlete.name}`}
                  hitSlop={8}
                  onPress={() => destinations.writeToGroup(athlete, athlete.attention[0] ?? null)}
                  testID={`coach-nonsharer-write-${athlete.user_id}`}
                >
                  <Text className="text-sm font-medium text-primary">{t('coachHome.action.writeToAthlete')}</Text>
                </Pressable>
              )}
            </View>
          ))}
        </View>
      )}

      <Text className="px-4 pb-4 pt-4 text-xs text-text-secondary">{t('coachHome.roster.footnote')}</Text>
    </View>
  );
}
