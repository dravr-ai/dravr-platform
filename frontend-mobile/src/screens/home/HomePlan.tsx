// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Home plan section — today's session enlarged with tomorrow under it; the week strip is HomeWeek's
// ABOUTME: A rest day and a day the plan never reached read differently; a tap on today opens a chat drafted about it

import React, { useMemo } from 'react';
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
  type TrainingPlanResponse,
  type WorkoutPlan,
} from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import { EmptyState, Section } from '../../components/ui';
import { useHomePreferences } from '../../hooks/useHome';
import { spacing, useThemeColors } from '../../constants/theme';
import { Figure } from '../chat/WorkoutPlanCard';
import { planDayDraft, planDayRouteDraft, sportLabel } from './homeFormat';

/** Everything the plan sections read, derived once from the plan and the athlete's today. */
interface HomeCalendar {
  today: string;
  todayLookup: PlanDayLookup;
  tomorrowLookup: PlanDayLookup;
  /** The phase covering today and which of its weeks today falls in, when the plan has one. */
  phaseWeek: { phase: PlanPhase; week: number } | null;
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
  // The week's Monday is checked here too: a today the calendar cannot place
  // is the plan failing to load, never a guessed day.
  mondayOf(today);
  return {
    today,
    todayLookup: planDayOn(plan, today),
    tomorrowLookup: planDayOn(plan, addCivilDays(today, 1)),
    phaseWeek: phaseOn(plan, today),
  };
}

/** "Run · 50 min · Z3" — a session in one line, its sport in the app language, its figure in mono. */
function SessionSummary({ day }: { day: PlanDay }) {
  const { t } = useTranslation();
  return (
    <Text className="text-sm text-text-secondary">
      {sportLabel(t, day.sport)}
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
  const colors = useThemeColors();
  const { todayLookup, tomorrowLookup, phaseWeek } = calendar;
  const phaseLabel =
    phaseWeek === null
      ? null
      : t('home.plan.phaseWeek', {
          phase: t(`plan.card.phase.${phaseWeek.phase.kind}`, { defaultValue: phaseWeek.phase.kind }),
          week: phaseWeek.week,
        });

  const routeDraft =
    todayLookup.kind === 'session' ? planDayRouteDraft(t, todayLookup.day, language) : null;

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
      {routeDraft !== null ? (
        <Pressable
          onPress={() => openDraft(routeDraft)}
          accessibilityRole="button"
          className="min-h-11 flex-row items-center self-start"
          testID="home-today-route"
        >
          <Feather name="map-pin" size={16} color={colors.tokens.primary} style={{ marginRight: spacing.xs }} />
          <Text className="text-sm font-medium text-primary">{t('home.plan.routeCta')}</Text>
        </Pressable>
      ) : null}
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
 * No active plan: say so, and offer to build one as a quiet ink link beside
 * a way to set the offer aside — not every athlete wants a plan, and a
 * suggestion they cannot get rid of becomes the page's loudest thing
 * (carnet#820). The choice is stored on the server, so the web honours it
 * too, and Settings brings it back. Set aside, the section keeps its one
 * plain sentence; while the choice is unknown, the links wait for it.
 */
function EmptyPlan({ openDraft }: { openDraft: OpenDraft }) {
  const { t } = useTranslation();
  const home = useHomePreferences();
  const preferences = home.preferences ?? (home.isError ? { plan_suggestion_hidden: false } : null);
  return (
    <View className="px-4" testID="home-plan-empty">
      <Text className="text-sm font-medium text-text-primary">{t('home.plan.emptyTitle')}</Text>
      {preferences !== null && !preferences.plan_suggestion_hidden ? (
        <>
          <Text className="mt-1 text-sm text-text-secondary">{t('home.plan.emptyBody')}</Text>
          <View className="mt-1 flex-row flex-wrap items-center">
            <Text
              className="min-h-11 py-3 pr-5 text-sm font-medium text-primary"
              onPress={() => openDraft(t('home.plan.buildDraft'))}
              accessibilityRole="button"
              testID="home-plan-build"
            >
              {t('home.plan.buildCta')}
            </Text>
            <Text
              className="min-h-11 py-3 text-sm text-text-secondary"
              onPress={() => home.update({ ...preferences, plan_suggestion_hidden: true })}
              accessibilityRole="button"
              accessibilityLabel={t('home.plan.hideSuggestionAria')}
              testID="home-plan-hide"
            >
              {t('home.plan.hideSuggestion')}
            </Text>
          </View>
        </>
      ) : null}
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
    </>
  );
}
