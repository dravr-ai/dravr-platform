// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Home week — a Monday-to-Sunday strip that pages to past and next weeks, expands to a month sheet, and folds next week
// ABOUTME: Past days show the workouts done, future days the plan's sessions; it renders without a plan and stops at stored history

import React, { useState } from 'react';
import { Pressable, Text, View } from 'react-native';
import { Feather } from '@expo/vector-icons';
import {
  addCivilDays,
  calendarDay,
  lastMonday,
  mondayOf,
  weekDays,
  type HomeActivity,
  type PlanWeek,
  type TrainingPlanResponse,
  type WorkoutPlan,
} from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import { EmptyState, Section } from '../../components/ui';
import { spacing, useThemeColors } from '../../constants/theme';
import { useHomeCalendar } from '../../hooks/useHomeCalendar';
import { WeekHeading } from '../chat/WorkoutPlanCard';
import { CalendarDayDetail, DayMarks, PlanDayButton } from './CalendarDayDetail';
import { HomeMonthSheet } from './HomeMonthSheet';
import { calendarCellLabel, civilDayMonth } from './calendarFormat';
import { civilDayOfMonth, civilLongDate, civilWeekdayNarrow } from './homeFormat';

const DAYS_PER_WEEK = 7;

type OpenDraft = (draft: string) => void;
type OpenActivity = (activity: HomeActivity) => void;

/** The round arrow and month buttons in the section's header. */
function HeaderButton({
  icon,
  label,
  disabled = false,
  onPress,
  testID,
}: {
  icon: React.ComponentProps<typeof Feather>['name'];
  label: string;
  disabled?: boolean;
  onPress: () => void;
  testID: string;
}) {
  const colors = useThemeColors();
  return (
    <Pressable
      onPress={onPress}
      disabled={disabled}
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled }}
      hitSlop={spacing.xs}
      className={`h-11 w-11 items-center justify-center ${disabled ? 'opacity-40' : ''}`}
      testID={testID}
    >
      <Feather name={icon} size={18} color={colors.text.secondary} />
    </Pressable>
  );
}

/**
 * The plan's week after this one: its heading with the focus the plan gave
 * it, folded by default so Home stays about today. Opening it lists its days,
 * each one a way into a chat about it.
 */
function NextWeek({
  week,
  index,
  currentIndex,
  openDraft,
}: {
  week: PlanWeek;
  index: number;
  currentIndex: number;
  openDraft: OpenDraft;
}) {
  const colors = useThemeColors();
  const [open, setOpen] = useState(false);
  return (
    <View className="px-4 mt-3">
      <Pressable
        onPress={() => setOpen((value) => !value)}
        accessibilityRole="button"
        accessibilityState={{ expanded: open }}
        className="min-h-11 flex-row items-center"
        testID="home-next-week"
      >
        <View className="flex-1">
          <WeekHeading week={week} index={index} currentIndex={currentIndex} />
        </View>
        <Feather
          name={open ? 'chevron-down' : 'chevron-right'}
          size={18}
          color={colors.text.secondary}
          style={{ marginLeft: spacing.sm }}
        />
      </Pressable>
      {open ? (
        <View testID="home-next-week-days">
          {week.days.map((day) => (
            <PlanDayButton key={day.date} day={day} openDraft={openDraft} />
          ))}
        </View>
      ) : null}
    </View>
  );
}

/**
 * The strip on the athlete's today, with or without a plan. It opens on this
 * week with today selected and pages a week at a time: back to the week
 * holding the first day the activity cache keeps, forward to the plan's last
 * week (next week without a plan).
 */
function WeekStrip({
  plan,
  today,
  openDraft,
  openActivity,
}: {
  plan: WorkoutPlan | null;
  today: string;
  openDraft: OpenDraft;
  openActivity: OpenActivity;
}) {
  const { t, language } = useTranslation();
  const thisMonday = mondayOf(today);
  const [monday, setMonday] = useState(thisMonday);
  const [selected, setSelected] = useState<string | null>(today);
  const [monthOpen, setMonthOpen] = useState(false);
  const days = weekDays(monday);
  const calendar = useHomeCalendar(days[0], days[DAYS_PER_WEEK - 1]);
  const answer = calendar.response;
  const historyStart = calendar.historyStart;
  const isThisWeek = monday === thisMonday;
  const shownWeek =
    answer?.plan_weeks?.find((week) => week.week_start === monday) ??
    plan?.weeks.find((week) => week.week_start === monday);
  const cells = days.map((date) => ({ date, day: answer === null ? null : calendarDay(answer, date) }));
  const open = cells.find((cell) => cell.date === selected)?.day ?? null;
  const currentIndex = plan?.weeks.findIndex((week) => week.current) ?? -1;
  const nextIndex =
    plan?.weeks.findIndex((week) => week.week_start === addCivilDays(thisMonday, DAYS_PER_WEEK)) ?? -1;

  const goTo = (target: string) => {
    setMonday(target);
    setSelected(target === thisMonday ? today : null);
  };

  return (
    <Section
      title={isThisWeek ? t('plan.card.thisWeek') : t('plan.card.weekOf', { date: civilDayMonth(monday, language) })}
      description={shownWeek?.focus ? shownWeek.focus : undefined}
      testID="home-section-week"
      actions={
        <>
          {!isThisWeek ? (
            <Pressable
              onPress={() => goTo(thisMonday)}
              accessibilityRole="button"
              className="min-h-11 justify-center px-1"
              testID="home-week-today"
            >
              <Text className="text-xs font-medium text-primary">{t('plan.card.thisWeek')}</Text>
            </Pressable>
          ) : null}
          <HeaderButton
            icon="chevron-left"
            label={t('home.calendar.previousWeek')}
            disabled={historyStart !== null && monday <= historyStart}
            onPress={() => goTo(addCivilDays(monday, -DAYS_PER_WEEK))}
            testID="home-week-previous"
          />
          <HeaderButton
            icon="chevron-right"
            label={t('home.calendar.nextWeek')}
            disabled={monday >= lastMonday(plan, today)}
            onPress={() => goTo(addCivilDays(monday, DAYS_PER_WEEK))}
            testID="home-week-next"
          />
          <HeaderButton
            icon="calendar"
            label={t('home.calendar.showMonth')}
            onPress={() => setMonthOpen(true)}
            testID="home-week-month"
          />
        </>
      }
    >
      <View className="px-4">
        {calendar.isError && answer === null ? (
          <EmptyState
            action={{ label: t('common.retry'), onPress: () => void calendar.refetch(), testID: 'home-week-retry' }}
            testID="home-week-failed"
          >
            {t('home.calendar.loadFailed')}
          </EmptyState>
        ) : (
          <View className="flex-row" testID="home-week-strip" accessibilityState={{ busy: answer === null }}>
            {cells.map(({ date, day }) => {
              const isToday = date === today;
              const isSelected = date === selected;
              return (
                <Pressable
                  key={date}
                  onPress={() => setSelected(date)}
                  accessibilityRole="button"
                  accessibilityState={{ selected: isSelected }}
                  accessibilityLabel={calendarCellLabel(t, language, date, day, today)}
                  className={`flex-1 items-center py-2 min-h-11 rounded-lg ${
                    isSelected ? 'bg-primary-container' : ''
                  } ${isToday ? 'border border-primary' : ''} ${day?.beforeHistory ? 'opacity-40' : ''}`}
                  testID={`home-week-day-${date}`}
                >
                  <Text className="text-xs text-text-secondary">{civilWeekdayNarrow(date, language)}</Text>
                  <Text
                    className={`text-sm font-mono tabular-nums ${
                      day === null || day.plan?.kind === 'uncovered' ? 'text-text-tertiary' : 'text-text-primary'
                    }`}
                  >
                    {civilDayOfMonth(date)}
                  </Text>
                  <View className="mt-1">{day !== null && !day.beforeHistory ? <DayMarks day={day} /> : <View className="h-2" />}</View>
                </Pressable>
              );
            })}
          </View>
        )}

        {open !== null ? (
          <CalendarDayDetail
            day={open}
            today={today}
            testID="home-week-detail"
            openDraft={openDraft}
            openActivity={openActivity}
          />
        ) : null}
        {answer !== null && historyStart !== null && monday <= historyStart ? (
          <Text className="mt-2 text-sm text-text-secondary" testID="home-week-history-start">
            {t('home.calendar.historyStart', { date: civilLongDate(historyStart, language) })}
          </Text>
        ) : null}
        {answer !== null && answer.plan_weeks === null ? (
          <Text className="mt-2 text-sm text-text-secondary" testID="home-week-no-plan">
            {t('home.calendar.noPlan')}
          </Text>
        ) : null}
      </View>
      {isThisWeek && plan !== null && nextIndex >= 0 ? (
        <NextWeek week={plan.weeks[nextIndex]} index={nextIndex} currentIndex={currentIndex} openDraft={openDraft} />
      ) : null}
      <HomeMonthSheet
        visible={monthOpen}
        plan={plan}
        today={today}
        startMonth={monday}
        onClose={() => setMonthOpen(false)}
        openDraft={(draft) => {
          setMonthOpen(false);
          openDraft(draft);
        }}
        openActivity={(activity) => {
          setMonthOpen(false);
          openActivity(activity);
        }}
      />
    </Section>
  );
}

interface HomeWeekProps {
  /** The plan read the Today section holds: null until it answers. */
  response: TrainingPlanResponse | null;
  openDraft: OpenDraft;
  openActivity: OpenActivity;
}

/**
 * Home's week. It reads the athlete's today from the plan answer the Today
 * section already holds — never a second clock — and draws nothing until
 * that answer arrives, which Today shows as loading or failed. An athlete
 * without a plan still gets the strip, with what they did on each day.
 */
export function HomeWeek({ response, openDraft, openActivity }: HomeWeekProps) {
  if (response === null) return null;
  try {
    mondayOf(response.today);
  } catch (error) {
    // A today that is not a calendar day is the Today section's load error.
    if (error instanceof RangeError) return null;
    throw error;
  }
  return (
    <WeekStrip
      key={response.today}
      plan={response.plan}
      today={response.today}
      openDraft={openDraft}
      openActivity={openActivity}
    />
  );
}
