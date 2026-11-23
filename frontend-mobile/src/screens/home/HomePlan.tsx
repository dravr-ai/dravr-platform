// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Home plan section — today's sessions enlarged with tomorrow's under them; the week strip is HomeWeek's
// ABOUTME: A rest day and a day the plan never reached read differently; a tap on a session opens a chat drafted about it

import React, { useMemo, useState } from 'react';
import { ActivityIndicator, Pressable, Text, View } from 'react-native';
import { Feather } from '@expo/vector-icons';
import {
  addCivilDays,
  DEFAULT_HOME_PREFERENCES,
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
import { planDayDraft, planDayRouteDraft, sportLabel, useDistanceUnit } from '@pierre/ui-logic';
import { healthCheckOutstanding } from '@pierre/shared-constants';
import { EmptyState, Section } from '../../components/ui';
import { HealthCheckSheet } from '../../components/HealthCheckSheet';
import { useHomePreferences } from '../../hooks/useHome';
import { useOnboardingStatus } from '../../hooks/useOnboardingStatus';
import { spacing, useThemeColors } from '../../constants/theme';
import { Figure } from '../chat/WorkoutPlanCard';

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

/** Opens a new chat with the given text in its composer. */
type OpenDraft = (draft: string) => void;

/**
 * One of today's entries, enlarged: the session with its sport, length and
 * intensity, or the rest day — a tap drafts a chat about that entry — then,
 * for a session in a sport with routes, the way to a route for it. A date
 * may hold several entries (carnet#1716); each is its own.
 */
function TodayEntry({ day, position, openDraft }: { day: PlanDay; position: number; openDraft: OpenDraft }) {
  const { t, language } = useTranslation();
  const unit = useDistanceUnit();
  const colors = useThemeColors();
  const routeDraft = planDayRouteDraft(t, language, day, unit);
  return (
    <View className={position > 0 ? 'mt-3' : undefined}>
      <Pressable
        onPress={() => openDraft(planDayDraft(t, language, day))}
        accessibilityRole="button"
        className="min-h-11 justify-center"
        testID={`home-today-day-${position}`}
      >
        {day.rest ? (
          <Text className="text-xl font-semibold text-text-primary">{t('chat.restDay')}</Text>
        ) : (
          <>
            <Text className="text-xl font-semibold text-text-primary">{day.workout}</Text>
            <SessionSummary day={day} />
          </>
        )}
      </Pressable>
      {routeDraft !== null ? (
        <Pressable
          onPress={() => openDraft(routeDraft)}
          accessibilityRole="button"
          // Two sessions on a day offer two of these: the hint says whose it is.
          accessibilityHint={day.workout}
          className="min-h-11 flex-row items-center self-start"
          testID={`home-today-route-${position}`}
        >
          <Feather name="map-pin" size={16} color={colors.tokens.primary} style={{ marginRight: spacing.xs }} />
          <Text className="text-sm font-medium text-primary">{t('home.plan.routeCta')}</Text>
        </Pressable>
      ) : null}
    </View>
  );
}

/** One of tomorrow's entries, as the short value beside its label: the session and its length, or rest. */
function TomorrowEntry({ day, position, openDraft }: { day: PlanDay; position: number; openDraft: OpenDraft }) {
  const { t, language } = useTranslation();
  let value: React.ReactNode;
  if (day.rest) {
    value = t('chat.restDay');
  } else if (day.duration_min !== undefined) {
    value = (
      <>
        {day.workout}
        {' · '}
        <Figure>{day.duration_min} min</Figure>
      </>
    );
  } else {
    value = day.workout;
  }
  return (
    <Pressable
      onPress={() => openDraft(planDayDraft(t, language, day))}
      accessibilityRole="button"
      className="min-h-11 justify-center"
      testID={`home-tomorrow-day-${position}`}
    >
      <Text className="text-sm text-text-primary" numberOfLines={1} testID={`home-tomorrow-value-${position}`}>
        {value}
      </Text>
    </Pressable>
  );
}

/**
 * Today, enlarged: each of its entries in order — a session with its sport,
 * length and intensity, or rest — then the phase and its week, and tomorrow's
 * entries underneath, one line each. A day outside the plan's shown weeks
 * says the plan does not cover it — never rest, because the plan did not say
 * rest.
 */
function TodayPlan({ calendar, openDraft }: { calendar: HomeCalendar; openDraft: OpenDraft }) {
  const { t } = useTranslation();
  const { todayLookup, tomorrowLookup, phaseWeek } = calendar;
  const phaseLabel =
    phaseWeek === null
      ? null
      : t('home.plan.phaseWeek', {
          phase: t(`plan.card.phase.${phaseWeek.phase.kind}`, { defaultValue: phaseWeek.phase.kind }),
          week: phaseWeek.week,
        });

  // No card: on the phone a group is its Section, and today is that group's
  // content (DESIGN.md "Phone"). The larger type is what sets it apart.
  return (
    <View className="px-4" testID="home-today">
      {todayLookup.kind === 'uncovered' ? (
        <Text className="text-base text-text-secondary" testID="home-today-uncovered">
          {t('home.plan.notCovered')}
        </Text>
      ) : (
        todayLookup.days.map((day, position) => (
          <TodayEntry key={`${day.date}-${position}`} day={day} position={position} openDraft={openDraft} />
        ))
      )}
      {phaseLabel !== null ? (
        <Text className="mt-2 text-sm text-text-secondary" testID="home-today-phase">
          {phaseLabel}
        </Text>
      ) : null}
      <View className="mt-3 pt-3 flex-row border-t border-border-faint" testID="home-tomorrow">
        <View className="min-h-11 justify-center mr-2">
          <Text className="text-sm font-medium text-text-secondary">{t('home.plan.tomorrow')}</Text>
        </View>
        <View className="flex-1">
          {tomorrowLookup.kind === 'uncovered' ? (
            <View className="min-h-11 justify-center">
              <Text className="text-sm text-text-primary" numberOfLines={1} testID="home-tomorrow-uncovered">
                {t('home.plan.notCovered')}
              </Text>
            </View>
          ) : (
            tomorrowLookup.days.map((day, position) => (
              <TomorrowEntry key={`${day.date}-${position}`} day={day} position={position} openDraft={openDraft} />
            ))
          )}
        </View>
      </View>
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
 *
 * No plan before the health check (carnet#1530): while the PAR-Q is still to
 * answer, « Construire mon plan » opens the health-check sheet first and
 * drafts the request once it is answered. Only this link waits on it; the
 * agent's plan tools are unchanged.
 */
function EmptyPlan({ openDraft }: { openDraft: OpenDraft }) {
  const { t } = useTranslation();
  const home = useHomePreferences();
  // Home is behind sign-in, so the shared status read is always on here.
  const onboarding = useOnboardingStatus(true);
  const [healthCheckOpen, setHealthCheckOpen] = useState(false);
  const preferences = home.preferences ?? (home.isError ? DEFAULT_HOME_PREFERENCES : null);
  const healthFirst = onboarding.data !== undefined && healthCheckOutstanding(onboarding.data.steps);
  const draftPlan = () => openDraft(t('home.plan.buildDraft'));
  return (
    <View className="px-4" testID="home-plan-empty">
      <Text className="text-sm font-medium text-text-primary">{t('home.plan.emptyTitle')}</Text>
      {preferences !== null && !preferences.plan_suggestion_hidden ? (
        <>
          <Text className="mt-1 text-sm text-text-secondary">
            {t(healthFirst ? 'home.plan.emptyBodyHealthFirst' : 'home.plan.emptyBody')}
          </Text>
          <HealthCheckSheet
            visible={healthCheckOpen}
            onClose={() => setHealthCheckOpen(false)}
            onAnswered={draftPlan}
          />
          <View className="mt-1 flex-row flex-wrap items-center">
            <Text
              className="min-h-11 py-3 pr-5 text-sm font-medium text-primary"
              onPress={() => (healthFirst ? setHealthCheckOpen(true) : draftPlan())}
              accessibilityRole="button"
              testID="home-plan-build"
            >
              {t('home.plan.buildCta')}
            </Text>
            <Text
              className="min-h-11 py-3 text-sm text-text-secondary"
              onPress={() => home.update({ plan_suggestion_hidden: true })}
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
