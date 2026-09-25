// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Home plan sections — today's session enlarged with tomorrow under it, then the Monday-to-Sunday strip and next week
// ABOUTME: A rest day and a day the plan never reached read differently; a tap on a day opens a chat drafted about it

import React, { useMemo, useState } from 'react';
import { ActivityIndicator, Pressable, Text, View } from 'react-native';
import { Feather } from '@expo/vector-icons';
import {
  addCivilDays,
  mondayOf,
  phaseWeekOn,
  planDayOn,
  type PlanDay,
  type PlanDayLookup,
  type PlanPhase,
  type PlanWeek,
  type TrainingPlanResponse,
  type WorkoutPlan,
} from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import { Button, EmptyState, Section } from '../../components/ui';
import { spacing, useThemeColors } from '../../constants/theme';
import { DayRow, Figure, WeekHeading } from '../chat/WorkoutPlanCard';
import {
  civilDayOfMonth,
  civilWeekdayLong,
  civilWeekdayNarrow,
  planDayDraft,
  type Translate,
} from './homeFormat';

const DAYS_PER_WEEK = 7;

/** One cell of the strip: a date and what the plan holds on it. */
interface StripDay {
  date: string;
  lookup: PlanDayLookup;
}

/** Everything the plan sections read, derived once from the plan and the athlete's today. */
interface HomeCalendar {
  today: string;
  todayLookup: PlanDayLookup;
  tomorrowLookup: PlanDayLookup;
  strip: StripDay[];
  /** The phase covering today and which of its weeks today falls in, when the plan has one. */
  phaseWeek: { phase: PlanPhase; week: number } | null;
  /** The shown week after the one covering today, for the "Next week" line. */
  nextWeek: { week: PlanWeek; index: number; currentIndex: number } | null;
}

/** The phase the server flagged current, else the one whose dates cover today. */
function phaseOn(plan: WorkoutPlan, today: string): { phase: PlanPhase; week: number } | null {
  const flagged = plan.phases.find((phase) => phase.current);
  const candidates = flagged ? [flagged, ...plan.phases] : plan.phases;
  for (const phase of candidates) {
    const week = phaseWeekOn(phase, today);
    if (week !== null) {
      return { phase, week };
    }
  }
  return null;
}

/**
 * Read the plan against today.
 *
 * @throws RangeError when `today` or a phase's dates are not calendar dates —
 *   a projection the page cannot place on the calendar at all, which the
 *   caller shows as the plan failing to load rather than as a guessed week.
 */
export function buildHomeCalendar(plan: WorkoutPlan, today: string): HomeCalendar {
  const monday = mondayOf(today);
  const strip = Array.from({ length: DAYS_PER_WEEK }, (_, offset) => {
    const date = addCivilDays(monday, offset);
    return { date, lookup: planDayOn(plan, date) };
  });

  const byCurrent = plan.weeks.findIndex((week) => week.current);
  const currentIndex =
    byCurrent >= 0 ? byCurrent : plan.weeks.findIndex((week) => week.week_start === monday);
  const next = currentIndex >= 0 ? plan.weeks[currentIndex + 1] : undefined;

  return {
    today,
    todayLookup: planDayOn(plan, today),
    tomorrowLookup: planDayOn(plan, addCivilDays(today, 1)),
    strip,
    phaseWeek: phaseOn(plan, today),
    nextWeek: next === undefined ? null : { week: next, index: currentIndex + 1, currentIndex },
  };
}

/** "Tempo run · run · 50 min · Z3" — a session in one line, its figure in mono. */
function SessionSummary({ day }: { day: PlanDay }) {
  return (
    <Text className="text-sm text-text-secondary">
      {day.sport}
      {day.duration_min !== undefined ? (
        <Text>
          {' · '}
          <Figure>{day.duration_min} min</Figure>
        </Text>
      ) : null}
      {day.intensity ? ` · ${day.intensity}` : ''}
    </Text>
  );
}

/** What tomorrow holds, as the short value beside its label: the session and its length, rest, or silence. */
function TomorrowValue({ lookup }: { lookup: PlanDayLookup }) {
  const { t } = useTranslation();
  let value: React.ReactNode;
  switch (lookup.kind) {
    case 'session':
      value =
        lookup.day.duration_min !== undefined ? (
          <>
            {lookup.day.workout}
            {' · '}
            <Figure>{lookup.day.duration_min} min</Figure>
          </>
        ) : (
          lookup.day.workout
        );
      break;
    case 'rest':
      value = t('chat.restDay');
      break;
    case 'uncovered':
      value = t('home.plan.notCovered');
      break;
  }
  return (
    <Text className="flex-1 text-sm text-text-primary" numberOfLines={1} testID="home-tomorrow-value">
      {value}
    </Text>
  );
}

/** Opens a new chat with the given text in its composer. */
type OpenDraft = (draft: string) => void;

/**
 * Today, enlarged: the session with its sport, length and intensity, the
 * phase and its week, and tomorrow on one line underneath. A rest day says
 * rest; a day outside the plan's shown weeks says the plan does not cover it
 * — never rest, because the plan did not say rest.
 */
function TodayPlan({ calendar, openDraft }: { calendar: HomeCalendar; openDraft: OpenDraft }) {
  const { t, language } = useTranslation();
  const { todayLookup, tomorrowLookup, phaseWeek } = calendar;
  const phaseLabel =
    phaseWeek === null
      ? null
      : t('home.plan.phaseWeek', {
          phase: t(`plan.card.phase.${phaseWeek.phase.kind}`, { defaultValue: phaseWeek.phase.kind }),
          week: phaseWeek.week,
        });

  const today =
    todayLookup.kind === 'uncovered' ? (
      <Text className="text-base text-text-secondary" testID="home-today-uncovered">
        {t('home.plan.notCovered')}
      </Text>
    ) : (
      <Pressable
        onPress={() => openDraft(planDayDraft(t, todayLookup.day, language))}
        accessibilityRole="button"
        className="min-h-11 justify-center"
        testID="home-today-day"
      >
        {todayLookup.kind === 'rest' ? (
          <Text className="text-xl font-semibold text-text-primary">{t('chat.restDay')}</Text>
        ) : (
          <>
            <Text className="text-xl font-semibold text-text-primary">{todayLookup.day.workout}</Text>
            <SessionSummary day={todayLookup.day} />
          </>
        )}
      </Pressable>
    );

  // No card: on the phone a group is its Section, and today is that group's
  // content (DESIGN.md §10). The larger type is what sets it apart.
  return (
    <View className="px-4" testID="home-today">
      {today}
      {phaseLabel !== null ? (
        <Text className="mt-2 text-sm text-text-secondary" testID="home-today-phase">
          {phaseLabel}
        </Text>
      ) : null}
      <Pressable
        onPress={
          tomorrowLookup.kind === 'uncovered'
            ? undefined
            : () => openDraft(planDayDraft(t, tomorrowLookup.day, language))
        }
        disabled={tomorrowLookup.kind === 'uncovered'}
        accessibilityRole={tomorrowLookup.kind === 'uncovered' ? undefined : 'button'}
        className="mt-3 pt-3 min-h-11 flex-row items-center border-t border-border-faint"
        testID="home-tomorrow"
      >
        <Text className="text-sm font-medium text-text-secondary mr-2">{t('home.plan.tomorrow')}</Text>
        <TomorrowValue lookup={tomorrowLookup} />
      </Pressable>
    </View>
  );
}

/**
 * The mark under a strip cell's date: a filled bar for a session, a hollow
 * one for rest, nothing where the plan is silent. Filled against hollow, not
 * one colour against another, so the difference survives any colour vision;
 * the cell's spoken label says it in words.
 */
function DayMark({ lookup }: { lookup: PlanDayLookup }) {
  switch (lookup.kind) {
    case 'session':
      return <View className="h-1 w-4 rounded-sm bg-primary" testID="home-week-mark-session" />;
    case 'rest':
      return <View className="h-1 w-4 rounded-sm border border-outline" testID="home-week-mark-rest" />;
    case 'uncovered':
      return <View className="h-1 w-4" />;
  }
}

/** What a strip cell is, in words, for a screen reader that cannot see the mark. */
function stripCellLabel(cell: StripDay, language: string, t: Translate): string {
  const when = `${civilWeekdayLong(cell.date, language)} ${civilDayOfMonth(cell.date)}`;
  switch (cell.lookup.kind) {
    case 'session':
      return `${when}, ${cell.lookup.day.workout}`;
    case 'rest':
      return `${when}, ${t('chat.restDay')}`;
    case 'uncovered':
      return `${when}, ${t('home.plan.notCovered')}`;
  }
}

/**
 * Monday to Sunday of the week covering today, today ringed. A tap selects a
 * day and shows its session — steps and fuel included — under the strip; a
 * tap on that session opens a chat about it.
 */
function WeekStrip({ calendar, openDraft }: { calendar: HomeCalendar; openDraft: OpenDraft }) {
  const { t, language } = useTranslation();
  const [selected, setSelected] = useState(calendar.today);
  const selectedCell = calendar.strip.find((cell) => cell.date === selected) ?? calendar.strip[0];

  return (
    <View className="px-4">
      <View className="flex-row" testID="home-week-strip">
        {calendar.strip.map((cell) => {
          const isToday = cell.date === calendar.today;
          const isSelected = cell.date === selectedCell.date;
          return (
            <Pressable
              key={cell.date}
              onPress={() => setSelected(cell.date)}
              accessibilityRole="button"
              accessibilityState={{ selected: isSelected }}
              accessibilityLabel={stripCellLabel(cell, language, t)}
              className={`flex-1 items-center py-2 min-h-11 rounded-lg ${
                isSelected ? 'bg-primary-container' : ''
              } ${isToday ? 'border border-primary' : ''}`}
              testID={`home-week-day-${cell.date}`}
            >
              <Text className="text-xs text-text-secondary">{civilWeekdayNarrow(cell.date, language)}</Text>
              <Text
                className={`text-sm font-mono tabular-nums ${
                  cell.lookup.kind === 'uncovered' ? 'text-text-tertiary' : 'text-text-primary'
                }`}
              >
                {civilDayOfMonth(cell.date)}
              </Text>
              <View className="mt-1">
                <DayMark lookup={cell.lookup} />
              </View>
            </Pressable>
          );
        })}
      </View>

      <View className="mt-2" testID="home-week-detail">
        {selectedCell.lookup.kind === 'uncovered' ? (
          <Text className="text-sm text-text-secondary py-2">
            <Figure>{selectedCell.date}</Figure> {t('home.plan.notCovered')}
          </Text>
        ) : (
          <PlanDayButton day={selectedCell.lookup.day} openDraft={openDraft} />
        )}
      </View>
    </View>
  );
}

/** A plan day drawn the way the chat card draws it, pressable into a drafted chat about it. */
function PlanDayButton({ day, openDraft }: { day: PlanDay; openDraft: OpenDraft }) {
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

/**
 * The next shown week: its heading with the focus the plan gave it, folded
 * by default so Home stays about today. Opening it lists its days, each one a
 * way into a chat about it.
 */
function NextWeek({
  next,
  openDraft,
}: {
  next: NonNullable<HomeCalendar['nextWeek']>;
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
          <WeekHeading week={next.week} index={next.index} currentIndex={next.currentIndex} />
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
          {next.week.days.map((day) => (
            <PlanDayButton key={day.date} day={day} openDraft={openDraft} />
          ))}
        </View>
      ) : null}
    </View>
  );
}

/**
 * No active plan: say so, and offer the one thing to do about it. Home is
 * built around the plan, so this is the page's one call to action and the
 * one filled button the view carries (the approved Home layout), where a
 * list's empty state is a sentence with an ink link.
 */
function EmptyPlan({ openDraft }: { openDraft: OpenDraft }) {
  const { t } = useTranslation();
  return (
    <View className="px-4" testID="home-plan-empty">
      <Text className="text-lg font-semibold text-text-primary">{t('home.plan.emptyTitle')}</Text>
      <Text className="mt-1 text-sm text-text-secondary">{t('home.plan.emptyBody')}</Text>
      <View className="mt-4 items-start">
        <Button
          title={t('home.plan.buildCta')}
          onPress={() => openDraft(t('home.plan.buildDraft'))}
          testID="home-plan-build"
        />
      </View>
    </View>
  );
}

interface HomePlanProps {
  /** Null until the first answer; stays null while a read is pending, paused offline, or failed. */
  response: TrainingPlanResponse | null;
  isError: boolean;
  onRetry: () => void;
  openDraft: OpenDraft;
}

/**
 * The Today section and, when there is a plan, the This week section.
 *
 * A failed read is an error with a retry, never the empty state: offering to
 * build a plan to an athlete whose plan the server failed to send would be
 * the wrong answer to the wrong question.
 */
export function HomePlan({ response, isError, onRetry, openDraft }: HomePlanProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();

  const calendar = useMemo(() => {
    if (response === null || response.plan === null) {
      return null;
    }
    try {
      return buildHomeCalendar(response.plan, response.today);
    } catch (error) {
      if (error instanceof RangeError) {
        return error;
      }
      throw error;
    }
  }, [response]);

  let today: React.ReactNode;
  if (response === null) {
    today = isError ? (
      <EmptyState
        action={{ label: t('common.retry'), onPress: onRetry, testID: 'home-plan-retry' }}
        testID="home-plan-error"
      >
        {t('home.plan.loadFailed')}
      </EmptyState>
    ) : (
      <View className="px-4 py-3 items-start" testID="home-plan-loading">
        <ActivityIndicator color={colors.tokens.primary} />
      </View>
    );
  } else if (response.plan === null) {
    today = <EmptyPlan openDraft={openDraft} />;
  } else if (calendar instanceof RangeError || calendar === null) {
    today = (
      <EmptyState
        action={{ label: t('common.retry'), onPress: onRetry, testID: 'home-plan-retry' }}
        testID="home-plan-error"
      >
        {t('home.plan.loadFailed')}
      </EmptyState>
    );
  } else {
    today = <TodayPlan calendar={calendar} openDraft={openDraft} />;
  }

  const week = calendar !== null && !(calendar instanceof RangeError) ? calendar : null;

  return (
    <>
      <Section title={t('chat.dayToday')} testID="home-section-today">
        {today}
        {isError && response !== null ? (
          <EmptyState
            action={{ label: t('common.retry'), onPress: onRetry, testID: 'home-plan-retry' }}
            testID="home-plan-error"
          >
            {t('home.plan.loadFailed')}
          </EmptyState>
        ) : null}
      </Section>
      {week !== null ? (
        <Section title={t('plan.card.thisWeek')} testID="home-section-week">
          <WeekStrip key={week.today} calendar={week} openDraft={openDraft} />
          {week.nextWeek !== null ? <NextWeek next={week.nextWeek} openDraft={openDraft} /> : null}
        </Section>
      ) : null}
    </>
  );
}
