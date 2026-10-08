// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: One open calendar day — the workouts done on it, each opening its own view, and the plan's session drafting a chat
// ABOUTME: Shared by the week strip and the month sheet; a day older than stored history says so instead of reading as empty

import React from 'react';
import { Pressable, Text, View } from 'react-native';
import { useTranslation } from '@pierre/i18n';
import type { CalendarDay, HomeActivity, PlanDay, PlanDayLookup } from '@pierre/shared-types';
import { DayRow, Figure } from '../chat/WorkoutPlanCard';
import { activityFigures, planDayDraft, sportLabel } from './homeFormat';

/**
 * The marks under a cell's date: a dot for a workout done, then a filled bar
 * for a planned session or a hollow one for rest, nothing where the plan is
 * silent. Filled against hollow, not one colour against another, so the
 * difference survives any colour vision; the spoken label says it in words.
 */
export function DayMarks({ day }: { day: CalendarDay }) {
  const done = day.activities.length > 0;
  const lookup: PlanDayLookup | null = day.plan;
  return (
    <View className="h-2 flex-row items-center gap-0.5">
      {done ? <View className="h-1.5 w-1.5 rounded-full bg-primary" testID="home-week-mark-done" /> : null}
      {lookup?.kind === 'session' ? (
        <View className={`h-1 rounded-sm bg-primary ${done ? 'w-2' : 'w-4'}`} testID="home-week-mark-session" />
      ) : null}
      {lookup?.kind === 'rest' ? (
        <View className={`h-1 rounded-sm border border-outline ${done ? 'w-2' : 'w-4'}`} testID="home-week-mark-rest" />
      ) : null}
    </View>
  );
}

/** A plan day drawn the way the chat card draws it, pressable into a drafted chat about it. */
export function PlanDayButton({ day, openDraft }: { day: PlanDay; openDraft: (draft: string) => void }) {
  const { t, language } = useTranslation();
  return (
    <Pressable
      onPress={() => openDraft(planDayDraft(t, day, language))}
      accessibilityRole="button"
      className="min-h-11 pb-2"
      testID={`home-plan-day-${day.date}`}
    >
      <DayRow day={day} />
    </Pressable>
  );
}

/** One workout of the day, the way into its own view. */
function ActivityLine({
  activity,
  openActivity,
}: {
  activity: HomeActivity;
  openActivity: (activity: HomeActivity) => void;
}) {
  const { t, language } = useTranslation();
  return (
    <Pressable
      onPress={() => openActivity(activity)}
      accessibilityRole="button"
      className="min-h-11 justify-center py-1"
      testID={`calendar-activity-${activity.provider}-${activity.id}`}
    >
      <Text className="text-sm font-medium text-text-primary" numberOfLines={1}>
        {activity.name}
      </Text>
      <Text className="text-xs text-text-secondary" numberOfLines={1}>
        {[sportLabel(t, activity.sport_type), ...activityFigures(t, activity, language)].join(' · ')}
      </Text>
    </Pressable>
  );
}

/** A small label over one half of the day: what was done, what was planned. */
function Part({ label }: { label: string }) {
  return <Text className="text-xs font-semibold text-text-secondary">{label}</Text>;
}

interface CalendarDayDetailProps {
  day: CalendarDay;
  /** The athlete's today: a day after it has nothing done to show. */
  today: string;
  testID: string;
  openDraft: (draft: string) => void;
  openActivity: (activity: HomeActivity) => void;
}

/**
 * The open day. Up to today it lists what was done; the plan's session for
 * it follows whenever the plan has one, so a past day reads as planned
 * against done. A day the plan never reached says so only from today on.
 */
export function CalendarDayDetail({ day, today, testID, openDraft, openActivity }: CalendarDayDetailProps) {
  const { t } = useTranslation();
  if (day.beforeHistory) {
    return (
      <View className="mt-2" testID={testID}>
        <Text className="text-sm text-text-secondary py-2">{t('home.calendar.notKept')}</Text>
      </View>
    );
  }
  const past = day.date <= today;
  const lookup = day.plan;
  const showPlan = lookup !== null && (lookup.kind !== 'uncovered' || !past);
  return (
    <View className="mt-2 gap-2" testID={testID}>
      {past ? (
        <View testID={`${testID}-done`}>
          {showPlan ? <Part label={t('home.calendar.done')} /> : null}
          {day.activities.length === 0 ? (
            <Text className="text-sm text-text-secondary py-2">{t('home.calendar.noActivity')}</Text>
          ) : (
            day.activities.map((activity) => (
              <ActivityLine key={`${activity.provider}-${activity.id}`} activity={activity} openActivity={openActivity} />
            ))
          )}
        </View>
      ) : null}
      {showPlan && lookup !== null ? (
        <View testID={`${testID}-planned`}>
          {past ? <Part label={t('home.calendar.planned')} /> : null}
          {lookup.kind === 'uncovered' ? (
            <Text className="text-sm text-text-secondary py-2">
              <Figure>{day.date}</Figure> {t('home.plan.notCovered')}
            </Text>
          ) : (
            <PlanDayButton day={lookup.day} openDraft={openDraft} />
          )}
        </View>
      ) : null}
      {lookup === null && !past ? (
        <Text className="text-sm text-text-secondary py-2">{t('home.calendar.noPlan')}</Text>
      ) : null}
    </View>
  );
}
