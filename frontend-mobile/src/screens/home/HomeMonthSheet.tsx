// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Home's week strip expanded to a month — a sheet with the month's grid: workouts done on past days, the plan's on the rest
// ABOUTME: Pages by month between stored history and the plan's last week; a tapped day opens the same detail the strip shows

import React, { useState } from 'react';
import { Pressable, ScrollView, Text, View } from 'react-native';
import { Feather } from '@expo/vector-icons';
import {
  calendarDay,
  firstOfMonth,
  lastMonday,
  monthGrid,
  shiftMonth,
  type HomeActivity,
  type WorkoutPlan,
} from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import { EmptyState, Sheet } from '../../components/ui';
import { useThemeColors } from '../../constants/theme';
import { useHomeCalendar } from '../../hooks/useHomeCalendar';
import { CalendarDayDetail, DayMarks } from './CalendarDayDetail';
import { calendarCellLabel, civilMonthTitle } from './calendarFormat';
import { civilDayOfMonth, civilLongDate, civilWeekdayNarrow } from './homeFormat';

interface HomeMonthSheetProps {
  visible: boolean;
  plan: WorkoutPlan | null;
  today: string;
  /** A day of the month the sheet opens on: the week the strip shows. */
  startMonth: string;
  onClose: () => void;
  openDraft: (draft: string) => void;
  openActivity: (activity: HomeActivity) => void;
}

/** The month's grid and its pager; mounted only while the sheet is open, so it reads nothing when closed. */
function MonthBody({
  plan,
  today,
  startMonth,
  openDraft,
  openActivity,
}: Omit<HomeMonthSheetProps, 'visible' | 'onClose'>) {
  const { t, language } = useTranslation();
  const colors = useThemeColors();
  const [month, setMonth] = useState(firstOfMonth(startMonth));
  const [selected, setSelected] = useState<string | null>(null);
  const grid = monthGrid(month);
  const first = grid[0][0];
  const last = grid[grid.length - 1][6];
  const calendar = useHomeCalendar(first, last);
  const answer = calendar.response;
  const historyStart = calendar.historyStart;
  const open = selected !== null && answer !== null ? calendarDay(answer, selected) : null;
  const canGoBack = historyStart === null || month > historyStart;
  const canGoForward = shiftMonth(month, 1) <= lastMonday(plan, today);

  const page = (months: number) => {
    setMonth((current) => shiftMonth(current, months));
    setSelected(null);
  };

  return (
    <ScrollView testID="home-month-scroll">
      <View className="flex-row items-center justify-between">
        <Text className="text-lg font-semibold text-text-primary" testID="home-month-title">
          {civilMonthTitle(month, language)}
        </Text>
        <View className="flex-row">
          <Pressable
            onPress={() => page(-1)}
            disabled={!canGoBack}
            accessibilityRole="button"
            accessibilityLabel={t('home.calendar.previousMonth')}
            accessibilityState={{ disabled: !canGoBack }}
            className={`h-11 w-11 items-center justify-center ${canGoBack ? '' : 'opacity-40'}`}
            testID="home-month-previous"
          >
            <Feather name="chevron-left" size={20} color={colors.text.secondary} />
          </Pressable>
          <Pressable
            onPress={() => page(1)}
            disabled={!canGoForward}
            accessibilityRole="button"
            accessibilityLabel={t('home.calendar.nextMonth')}
            accessibilityState={{ disabled: !canGoForward }}
            className={`h-11 w-11 items-center justify-center ${canGoForward ? '' : 'opacity-40'}`}
            testID="home-month-next"
          >
            <Feather name="chevron-right" size={20} color={colors.text.secondary} />
          </Pressable>
        </View>
      </View>
      <View className="flex-row mt-2" importantForAccessibility="no-hide-descendants">
        {grid[0].map((date) => (
          <Text key={date} className="flex-1 text-center text-xs text-text-secondary">
            {civilWeekdayNarrow(date, language)}
          </Text>
        ))}
      </View>
      {calendar.isError && answer === null ? (
        <EmptyState
          action={{ label: t('common.retry'), onPress: () => void calendar.refetch(), testID: 'home-month-retry' }}
          testID="home-month-failed"
        >
          {t('home.calendar.loadFailed')}
        </EmptyState>
      ) : (
        <View testID="home-month-grid" accessibilityState={{ busy: answer === null }}>
          {grid.map((week) => (
            <View key={week[0]} className="flex-row">
              {week.map((date) => {
                const day = answer === null ? null : calendarDay(answer, date);
                const inMonth = date.slice(0, 7) === month.slice(0, 7);
                const isToday = date === today;
                const isSelected = date === selected;
                return (
                  <Pressable
                    key={date}
                    onPress={() => setSelected(date)}
                    accessibilityRole="button"
                    accessibilityState={{ selected: isSelected }}
                    accessibilityLabel={calendarCellLabel(t, language, date, day, today)}
                    className={`flex-1 items-center py-1.5 min-h-11 rounded-lg ${
                      isSelected ? 'bg-primary-container' : ''
                    } ${isToday ? 'border border-primary' : ''} ${
                      day?.beforeHistory ? 'opacity-30' : inMonth ? '' : 'opacity-50'
                    }`}
                    testID={`home-month-day-${date}`}
                  >
                    <Text className="text-sm font-mono tabular-nums text-text-primary">{civilDayOfMonth(date)}</Text>
                    <View className="mt-1">
                      {day !== null && !day.beforeHistory ? <DayMarks day={day} /> : <View className="h-2" />}
                    </View>
                  </Pressable>
                );
              })}
            </View>
          ))}
        </View>
      )}
      {open !== null ? (
        <CalendarDayDetail
          day={open}
          today={today}
          testID="home-month-detail"
          openDraft={openDraft}
          openActivity={openActivity}
        />
      ) : null}
      {answer !== null && historyStart !== null && first <= historyStart ? (
        <Text className="mt-2 text-sm text-text-secondary" testID="home-month-history-start">
          {t('home.calendar.historyStart', { date: civilLongDate(historyStart, language) })}
        </Text>
      ) : null}
      {answer !== null && answer.plan_weeks === null ? (
        <Text className="mt-2 text-sm text-text-secondary" testID="home-month-no-plan">
          {t('home.calendar.noPlan')}
        </Text>
      ) : null}
    </ScrollView>
  );
}

/**
 * The month sheet. It opens on the month of the week the strip shows and
 * reads that month's whole-week grid in one calendar read; a day outside the
 * month is drawn faint, a day older than stored history fainter still, and a
 * sentence under the grid says where history starts rather than leave those
 * days to read as rest.
 */
export function HomeMonthSheet({ visible, onClose, ...body }: HomeMonthSheetProps) {
  return (
    <Sheet visible={visible} onClose={onClose} testID="home-month-sheet" backdropTestID="home-month-backdrop">
      {visible ? <MonthBody {...body} /> : null}
    </Sheet>
  );
}
