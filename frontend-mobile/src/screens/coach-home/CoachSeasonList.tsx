// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Board E on the phone — La saison as a countdown list: each athlete's phases from today to the A race, A races ahead by date
// ABOUTME: Dates and phases only, never form or load; an athlete with no A race ahead is under "Sans course A" with the state that says why

import React, { useMemo } from 'react';
import { Pressable, Text, View } from 'react-native';
import { COACH_DAY_UNIT_KEYS, COACH_PHASE_LABEL_KEY, COACH_SEASON_STATE_KEY, countKey } from '@pierre/shared-constants';
import { formatList } from '@pierre/chat-utils';
import { useTranslation } from '@pierre/i18n';
import { buildSeason, civilShortDate, countdownBars, nextPhase, type SeasonRow } from '@pierre/ui-logic';
import type { CoachRosterAthlete } from '@pierre/shared-types';
import { EmptyState, StatusDot } from '../../components/ui';
import { GroupLabel, phaseFill } from './parts';

interface CoachSeasonListProps {
  athletes: CoachRosterAthlete[];
  today: string;
  onOpen: (athleteId: string) => void;
}

/** The line beside an athlete's name: their A race, or the state their season is in. */
function useSubline() {
  const { t, language } = useTranslation();
  return (row: SeasonRow): string => {
    if (row.state === 'a_race_ahead') return row.raceName ?? '';
    const date =
      row.state === 'nothing_since_last_race'
        ? row.raceDate
        : row.state === 'plan_ended'
          ? (row.athlete.season?.ends ?? null)
          : null;
    return t(COACH_SEASON_STATE_KEY[row.state], date === null ? {} : { date: civilShortDate(date, language) });
  };
}

function SeasonRowView({ row, today, onOpen }: { row: SeasonRow; today: string; onOpen: (id: string) => void }) {
  const { t, language } = useTranslation();
  const subline = useSubline();
  const bars = countdownBars(row.athlete, today);
  const next = nextPhase(row.athlete, today);
  const plan = row.athlete.plan;
  let note = '';
  if (row.noDataSince !== null) {
    note = t('coachHome.seasonView.noData', { date: civilShortDate(row.noDataSince, language) });
  } else if (next !== null) {
    note = t('coachHome.seasonView.phaseIn', {
      phase: t(COACH_PHASE_LABEL_KEY[next.kind]),
      days: t(countKey(next.inDays, language, COACH_DAY_UNIT_KEYS), { count: next.inDays }),
    });
  } else if (plan?.phase) {
    note = `${t(COACH_PHASE_LABEL_KEY[plan.phase])} · ${t('coachHome.detail.planWeek', { week: plan.week ?? 1, total: plan.weeks_total })}`;
  }
  const line = subline(row);
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`${row.athlete.name}, ${line}`}
      onPress={() => onOpen(row.athlete.user_id)}
      testID={`coach-season-row-${row.athlete.user_id}`}
      className="gap-1.5 border-b border-border-faint py-2.5 active:bg-surface-container-low"
    >
      <View className="flex-row items-baseline gap-2">
        <Text className="text-sm font-semibold text-text-primary">{row.athlete.name}</Text>
        <Text className="min-w-0 flex-1 text-xs text-text-secondary" numberOfLines={1} testID="coach-season-subline">
          {line}
        </Text>
        {row.daysToRace !== null && (
          <Text className="font-mono text-xs text-text-secondary" testID={`coach-season-countdown-${row.athlete.user_id}`}>
            {t('coachHome.detail.countdown', { days: row.daysToRace })}
          </Text>
        )}
      </View>
      {bars.length > 0 && (
        <View
          accessibilityElementsHidden
          importantForAccessibility="no-hide-descendants"
          testID={`coach-season-bars-${row.athlete.user_id}`}
          className="h-2.5 flex-row gap-0.5"
        >
          {bars.map((bar, index) => (
            <View
              key={`${bar.kind}-${index}`}
              className={`rounded-sm ${phaseFill(bar.kind)}`}
              // A band's width is its share of the days to the race, so it is measured, not classed.
              style={{ flex: bar.days }}
            />
          ))}
        </View>
      )}
      {/* A status is an 8px dot beside the word that says it, never red ink (DESIGN.md §10). */}
      {note !== '' && row.noDataSince !== null && (
        <View className="flex-row items-center gap-1.5" testID={`coach-season-nodata-${row.athlete.user_id}`}>
          <StatusDot tone="warning" />
          <Text className="min-w-0 flex-1 text-xs text-text-secondary">{note}</Text>
        </View>
      )}
      {note !== '' && row.noDataSince === null && <Text className="text-xs text-text-secondary">{note}</Text>}
    </Pressable>
  );
}

/** Board E. */
export function CoachSeasonList({ athletes, today, onOpen }: CoachSeasonListProps) {
  const { t, language } = useTranslation();
  const view = useMemo(() => buildSeason(athletes, today, 'season'), [athletes, today]);
  const nothing = view.ahead.length === 0 && view.without.length === 0;

  const section = (title: string, rows: SeasonRow[], testID: string) => (
    <View className="px-4" testID={testID}>
      <GroupLabel count={rows.length}>{title}</GroupLabel>
      {rows.map((row) => (
        <SeasonRowView key={row.athlete.user_id} row={row} today={today} onOpen={onOpen} />
      ))}
    </View>
  );

  return (
    <View testID="coach-season">
      <View className="px-4 pt-3">
        <Text accessibilityRole="header" className="font-display text-xl font-semibold text-text-primary">
          {t('coachHome.seasonView.title')}
        </Text>
        <Text className="pt-1 text-xs font-medium text-text-secondary">{t('coachHome.seasonView.ahead')}</Text>
      </View>
      {nothing && <EmptyState testID="coach-season-empty">{t('coachHome.seasonView.empty')}</EmptyState>}
      {view.ahead.length > 0 && section(t('coachHome.seasonView.sectionRace'), view.ahead, 'coach-season-ahead')}
      {view.without.length > 0 && section(t('coachHome.seasonView.sectionNone'), view.without, 'coach-season-without')}
      {view.hidden.length > 0 && (
        <Text className="px-4 pb-4 pt-4 text-sm text-text-secondary" testID="coach-season-hidden">
          {t('coachHome.seasonView.noShare', { names: formatList(view.hidden.map((a) => a.name), language) })}
        </Text>
      )}
    </View>
  );
}
