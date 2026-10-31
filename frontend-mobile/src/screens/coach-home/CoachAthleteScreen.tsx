// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: One athlete of the coach's roster — why they are here, then their form band, plan, sources and what the agent said in the group
// ABOUTME: Evidence lines each carry their source and time; form is the server's band on the athlete's own scale and nothing else

import React, { useMemo } from 'react';
import { ActivityIndicator, ScrollView, Text, View } from 'react-native';
import { Stack, useLocalSearchParams } from 'expo-router';
import type { CoachAttentionReason, CoachRosterAthlete } from '@pierre/shared-types';
import {
  COACH_ACTION_LABEL_KEY,
  COACH_AGENT_COUNT_KEYS,
  COACH_PHASE_LABEL_KEY,
  COACH_UNVERIFIED_KEYS,
  FORM_BAND_LABEL_KEY,
  countKey,
} from '@pierre/shared-constants';
import { formatList } from '@pierre/chat-utils';
import { useTranslation } from '@pierre/i18n';
import {
  buildQueue,
  civilDayNumber,
  civilShortDate,
  daysBetween,
  describeLine,
  describeSource,
  formatSyncTime,
  localToday,
  providerName,
  sourceChips,
  type QueueEntry,
} from '@pierre/ui-logic';
import { Button, EmptyState, Row, Section, StatusDot } from '../../components/ui';
import { useThemeColors } from '../../constants/theme';
import { useCoachRoster } from '../../hooks/useCoachRoster';
import { AthleteAvatar, FormStrip, phaseFill, phaseInk } from './parts';
import { useCoachDestinations, type CoachDestinations } from './useCoachDestinations';
import { useTreatment } from './useTreatment';

/** One reason and the lines of evidence behind it. */
function ReasonBlock({ reason, lead }: { reason: CoachAttentionReason; lead: boolean }) {
  const { t, language } = useTranslation();
  return (
    <View className="gap-2 px-4" testID={`coach-reason-${reason.id}`}>
      <Text className={lead ? 'text-base text-text-primary' : 'text-sm font-medium text-text-secondary'}>
        {describeLine(t, language, reason.key, reason.params)}
      </Text>
      {reason.evidence.map((line, index) => (
        <View key={`${line.key}-${index}`} className="gap-0.5 border-t border-border-faint py-2" testID="coach-evidence">
          <Text className="text-sm text-text-primary">{describeLine(t, language, line.key, line.params)}</Text>
          <Text className="font-mono text-xs text-text-secondary" testID="coach-evidence-source">
            {describeSource(t, language, line.source, line.at)}
          </Text>
        </View>
      ))}
    </View>
  );
}

function FormBlock({ athlete }: { athlete: CoachRosterAthlete }) {
  const { t, language } = useTranslation();
  const training = athlete.training;
  const band = training?.form?.band ?? 'insufficient_history';
  const cut = training !== undefined && (training.stale || training.needs_reconnect.length > 0);
  let note: string;
  if (band === 'insufficient_history') note = t('coachHome.detail.formNoteInsufficient');
  else if (cut && training?.form) note = t('coachHome.detail.formNoteStale', { date: civilShortDate(training.form.as_of, language) });
  else if (training?.ctl !== null && training?.ctl !== undefined) note = t('coachHome.detail.formNote', { ctl: training.ctl });
  else note = t('coachHome.detail.formNoteNoCtl');
  return (
    <Section title={t('coachHome.detail.form')} testID="coach-form-block">
      <View className="gap-2 px-4">
        <Text className="text-sm text-text-primary" testID="coach-form-word">
          {t(FORM_BAND_LABEL_KEY[band])}
        </Text>
        <FormStrip band={band} size="md" muted={cut} />
        <View className="flex-row justify-between" accessibilityElementsHidden importantForAccessibility="no-hide-descendants">
          <Text className="text-xs text-text-secondary">{t(FORM_BAND_LABEL_KEY.deep_fatigue)}</Text>
          <Text className="text-xs text-text-secondary">{t(FORM_BAND_LABEL_KEY.detraining)}</Text>
        </View>
        <Text className="text-xs text-text-secondary">{note}</Text>
      </View>
    </Section>
  );
}

function PlanBlock({ athlete, today }: { athlete: CoachRosterAthlete; today: string }) {
  const { t, language } = useTranslation();
  const plan = athlete.plan;
  const season = athlete.season;
  const race = season?.state === 'a_race_ahead' && season.race ? season.race : plan?.goal_race;
  const toRace = race === undefined ? null : daysBetween(today, race.date);

  if (plan === undefined || plan.phases.length === 0) {
    const draft = athlete.plan_thread?.draft_updated_at !== null && athlete.plan_thread?.draft_updated_at !== undefined;
    return (
      <Section title={t('coachHome.detail.plan')} testID="coach-plan-block">
        <Text className="px-4 text-sm text-text-secondary">
          {draft ? t('coachHome.detail.planDraftOnly') : t('coachHome.detail.planNone')}
        </Text>
      </Section>
    );
  }

  const first = civilDayNumber(plan.phases[0].start);
  const lastPhase = plan.phases[plan.phases.length - 1];
  const last = civilDayNumber(lastPhase.end ?? lastPhase.start);
  const span = last - first;
  const todayPct = span > 0 ? Math.min(100, Math.max(0, ((civilDayNumber(today) - first) / span) * 100)) : 0;
  const note: string[] = [];
  if (plan.week !== null) note.push(t('coachHome.detail.planWeek', { week: plan.week, total: plan.weeks_total }));
  if (plan.pushed_to.length > 0) note.push(t('coachHome.detail.pushedTo', { platforms: formatList(plan.pushed_to.map(providerName), language) }));

  return (
    <Section title={t('coachHome.detail.plan')} testID="coach-plan-block">
      <View className="gap-2 px-4">
        <View className="flex-row flex-wrap items-baseline gap-2">
          <Text className="text-sm text-text-primary">
            {season?.state === 'a_race_ahead' && race !== undefined
              ? `${race.name} · ${civilShortDate(race.date, language)}`
              : t('coachHome.detail.noARace')}
          </Text>
          {season?.state === 'a_race_ahead' && toRace !== null && toRace >= 0 && (
            <Text className="font-mono text-xs text-text-secondary" testID="coach-countdown">
              {t('coachHome.detail.countdown', { days: toRace })}
            </Text>
          )}
        </View>
        <View
          accessible
          accessibilityRole="image"
          accessibilityLabel={plan.phase ? t(COACH_PHASE_LABEL_KEY[plan.phase]) : t('coachHome.detail.plan')}
          className="flex-row gap-[3px]"
        >
          {plan.phases.map((phase, index) => {
            const active = phase.kind === plan.phase;
            return (
              <View
                key={`${phase.kind}-${index}`}
                // A phase is as wide as it is long, so its share of the row is measured, not classed.
                style={{ flex: Math.max(1, phase.weeks) }}
                className={`h-[22px] min-w-0 justify-center overflow-hidden rounded-md px-1.5 ${
                  active ? 'bg-primary-container' : phaseFill(phase.kind)
                }`}
              >
                <Text
                  numberOfLines={1}
                  className={`text-xs font-medium ${active ? 'text-on-primary-container' : phaseInk(phase.kind)}`}
                >
                  {t(COACH_PHASE_LABEL_KEY[phase.kind])}
                </Text>
              </View>
            );
          })}
          <View
            accessibilityElementsHidden
            importantForAccessibility="no-hide-descendants"
            testID="coach-today-line"
            // Today's place on the season is a share of its width, so it is measured, not classed.
            style={{ left: `${todayPct}%` }}
            className="absolute -bottom-1 -top-1 w-0.5 rounded-sm bg-on-surface"
          />
        </View>
        {note.length > 0 && <Text className="text-xs text-text-secondary">{note.join(' · ')}</Text>}
      </View>
    </Section>
  );
}

function SourcesBlock({ athlete }: { athlete: CoachRosterAthlete }) {
  const { t, language } = useTranslation();
  const chips = sourceChips(athlete);
  return (
    <Section title={t('coachHome.detail.sources')} testID="coach-sources-block">
      <View className="gap-1 px-4">
        {chips.length === 0 && <Text className="text-sm text-text-secondary">{t('coachHome.detail.sourceNone')}</Text>}
        {chips.map((chip) => (
          <View key={chip.provider} className="min-h-[32px] flex-row items-center gap-2.5" testID={`coach-source-${chip.provider}`}>
            <StatusDot tone={chip.state === 'ok' ? 'success' : 'warning'} />
            <Text className="text-sm font-medium text-text-primary">{providerName(chip.provider)}</Text>
            <Text className="ml-auto text-xs text-text-secondary">
              {chip.state === 'reconnect'
                ? t('coachHome.detail.sourceReconnect')
                : chip.lastDay !== null
                  ? t('coachHome.detail.sourceLast', { date: civilShortDate(chip.lastDay, language) })
                  : t('coachHome.detail.sourceConnected')}
            </Text>
          </View>
        ))}
      </View>
    </Section>
  );
}

/**
 * What the agent said in the coach's groups this week. The roster carries the
 * week's counts and how many figures no source backs; the words of each reply
 * are read in the reply review, which the block opens on this athlete.
 */
function AgentBlock({ athlete, destinations }: { athlete: CoachRosterAthlete; destinations: CoachDestinations }) {
  const { t, language } = useTranslation();
  const replies = athlete.agent_replies;
  const has = replies !== undefined && replies.count > 0;
  return (
    <Section title={t('coachHome.detail.agent')} testID="coach-agent-block">
      <View className="gap-2 px-4">
        {!has && <Text className="text-sm text-text-secondary">{t('coachHome.detail.agentNone')}</Text>}
        {has && (
          <>
            <Text className="text-xs text-text-secondary">
              {t(countKey(replies.count, language, COACH_AGENT_COUNT_KEYS), { count: replies.count, verified: replies.verified })}
              {replies.capped ? ` · ${t('coachHome.detail.agentCapped')}` : ''}
            </Text>
            {/* A status is an 8px dot beside the word that says it (DESIGN.md §10). */}
            <View testID="coach-agent-verdict" className="flex-row items-center gap-1.5">
              <StatusDot tone={replies.unmatched_figures > 0 ? 'warning' : 'success'} />
              <Text className="text-xs font-medium text-text-primary">
                {replies.unmatched_figures > 0
                  ? t(countKey(replies.unmatched_figures, language, COACH_UNVERIFIED_KEYS), { count: replies.unmatched_figures })
                  : t('coachHome.detail.verdictVerified')}
              </Text>
            </View>
          </>
        )}
      </View>
      {has && (
        <Row
          title={t('coachReview.nav')}
          compact
          last
          onPress={() => destinations.openReplies(athlete)}
          testID="coach-agent-replies-link"
        />
      )}
    </Section>
  );
}

interface ActionsProps {
  athlete: CoachRosterAthlete;
  entry: QueueEntry | null;
  treatedReason: CoachAttentionReason | null;
  destinations: CoachDestinations;
  onTreat: () => void;
  onUndo: () => void;
}

/** The primary action the reason names, the group's room, and "Traité" or its undo. */
function Actions({ athlete, entry, treatedReason, destinations, onTreat, onUndo }: ActionsProps) {
  const { t } = useTranslation();
  const reason = entry?.reason ?? null;
  const primary = reason?.action ?? null;
  const showGroup = athlete.groups.length > 0 && primary !== 'write_to_athlete';
  const busy = destinations.pendingAthleteId === athlete.user_id;
  return (
    <View className="gap-2 px-4" testID="coach-actions">
      {primary !== null && reason !== null && (
        <Button
          title={t(COACH_ACTION_LABEL_KEY[primary])}
          disabled={busy}
          loading={busy}
          fullWidth
          testID="coach-primary-action"
          onPress={() => destinations.perform(primary, athlete, reason)}
        />
      )}
      {showGroup && (
        <Button
          title={t('coachHome.action.writeToGroup')}
          variant="secondary"
          fullWidth
          testID="coach-write-group"
          onPress={() => destinations.writeToGroup(athlete, reason)}
        />
      )}
      {reason !== null && <Button title={t('coachHome.action.treated')} variant="ghost" fullWidth testID="coach-treat" onPress={onTreat} />}
      {treatedReason !== null && (
        <Button title={t('coachHome.action.undo')} variant="ghost" fullWidth testID="coach-undo" onPress={onUndo} />
      )}
    </View>
  );
}

/** A roster athlete's detail, opened from any of Coach Home's three lists. */
export function CoachAthleteScreen() {
  const { t, language } = useTranslation();
  const colors = useThemeColors();
  const { athleteId } = useLocalSearchParams<{ athleteId: string }>();
  const { roster, isLoading, isError, refetch } = useCoachRoster(true);
  const destinations = useCoachDestinations();
  const { treat, undo } = useTreatment();
  const today = useMemo(() => localToday(), []);

  const athlete = roster?.athletes.find((a) => a.user_id === athleteId);
  const queue = useMemo(() => (roster === undefined ? null : buildQueue(roster.athletes)), [roster]);
  const entry = useMemo(
    () => (queue === null || athlete === undefined ? null : [...queue.today, ...queue.week].find((e) => e.athlete.user_id === athlete.user_id) ?? null),
    [queue, athlete],
  );
  const treatedReason = useMemo(
    () => (queue === null || athlete === undefined ? null : queue.treated.find((e) => e.athlete.user_id === athlete.user_id)?.reason ?? null),
    [queue, athlete],
  );

  if (athlete === undefined) {
    return (
      <View className="flex-1 bg-background-primary" testID="coach-athlete-missing">
        <Stack.Screen options={{ title: '' }} />
        {isLoading ? (
          <View className="items-center py-10">
            <ActivityIndicator color={colors.tokens.primary} />
          </View>
        ) : (
          <EmptyState action={isError ? { label: t('common.retry'), onPress: refetch } : undefined}>
            {isError ? t('coachHome.loadFailed') : t('coachHome.queue.selectAthlete')}
          </EmptyState>
        )}
      </View>
    );
  }

  const sharing = athlete.status === 'shared' && athlete.training !== undefined;

  return (
    <View className="flex-1 bg-background-primary" testID="coach-athlete">
      <Stack.Screen options={{ title: athlete.name }} />
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerClassName="gap-8 pb-10 pt-4" testID="coach-athlete-scroll">
        <View className="flex-row items-center gap-3 px-4">
          <AthleteAvatar userId={athlete.user_id} name={athlete.name} size="lg" muted={!sharing} />
          <View className="min-w-0 flex-1">
            <Text accessibilityRole="header" className="font-display text-xl font-semibold text-text-primary" numberOfLines={2}>
              {athlete.name}
            </Text>
            <Text className="text-xs text-text-secondary" numberOfLines={1}>
              {athlete.groups.map((g) => g.name).join(' · ')}
            </Text>
          </View>
        </View>

        <Actions
          athlete={athlete}
          entry={entry}
          treatedReason={treatedReason}
          destinations={destinations}
          onTreat={() => entry !== null && treat(athlete, entry.reason)}
          onUndo={() => treatedReason !== null && undo(athlete, treatedReason)}
        />

        {entry !== null && (
          <Section title={t('coachHome.detail.why')} testID="coach-why">
            <View className="gap-4">
              <ReasonBlock reason={entry.reason} lead />
              {entry.others.map((other) => (
                <ReasonBlock key={other.id} reason={other} lead={false} />
              ))}
            </View>
          </Section>
        )}

        {entry === null && treatedReason !== null && (
          <Section title={t('coachHome.detail.why')} testID="coach-treated-note">
            <View className="gap-3">
              <ReasonBlock reason={treatedReason} lead />
              <Text className="px-4 text-xs text-text-secondary">
                {t('coachHome.detail.treatedNote')}
                {treatedReason.dismissed_at !== null ? ` · ${formatSyncTime(treatedReason.dismissed_at, language)}` : ''}
              </Text>
            </View>
          </Section>
        )}

        {entry === null && treatedReason === null && sharing && (
          <EmptyState testID="coach-calm">{t('coachHome.detail.calm')}</EmptyState>
        )}

        {sharing && (
          <>
            <FormBlock athlete={athlete} />
            <PlanBlock athlete={athlete} today={today} />
            <SourcesBlock athlete={athlete} />
            <AgentBlock athlete={athlete} destinations={destinations} />
          </>
        )}

        {!sharing && (
          <Text className="px-4 text-sm text-text-secondary" testID="coach-hidden-note">
            {t('coachHome.detail.hidden')}
          </Text>
        )}
      </ScrollView>
    </View>
  );
}
