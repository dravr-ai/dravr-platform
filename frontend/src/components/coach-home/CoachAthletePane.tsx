// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A's right pane — why this athlete is here, then their form, plan, sources and what the agent said in the group
// ABOUTME: Evidence lines each carry their source and time; form is the server's band on the athlete's own scale and nothing else

import { useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Check } from 'lucide-react';
import { clsx } from 'clsx';
import {
  VERDICT_STATUS_TONE,
  summarizeVerdicts,
  verdictToneAlerts,
  type CoachAttentionReason,
  type CoachRosterAthlete,
} from '@pierre/shared-types';
import {
  COACH_ACTION_LABEL_KEY,
  COACH_AGENT_COUNT_KEYS,
  COACH_PHASE_LABEL_KEY,
  COACH_UNVERIFIED_KEYS,
  FORM_BAND_LABEL_KEY,
  VERDICT_CHIP_SEPARATOR,
  countKey,
  verdictChipLabel,
} from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import {
  civilDayNumber,
  civilShortDate,
  daysBetween,
  describeLine,
  describeSource,
  formatSyncTime,
  providerName,
  sourceChipWords,
  sourceChips,
  trainingCut,
  type QueueEntry,
} from '@pierre/ui-logic';
import { coachRepliesApi } from '../../services/api';
import { QUERY_KEYS } from '../../constants/queryKeys';
import { Button, EmptyState } from '../ui';
import { AthleteAvatar, FormStrip, StatusDot } from './parts';
import { phaseClass } from './phaseStyle';
import { PhaseBand } from './PhaseBand';
import type { CoachDestinations } from './useCoachDestinations';

interface CoachAthletePaneProps {
  athlete: CoachRosterAthlete;
  /** The athlete's place in the queue, when they have one open. */
  entry: QueueEntry | null;
  /** Their dismissed reason, when every reason was marked handled. */
  treatedReason: CoachAttentionReason | null;
  today: string;
  destinations: CoachDestinations;
  onTreat: (athlete: CoachRosterAthlete, reason: CoachAttentionReason) => void;
  onUndo: (athlete: CoachRosterAthlete, reason: CoachAttentionReason) => void;
}

function Heading({ children }: { children: React.ReactNode }) {
  return <h3 className="font-sans text-sm font-semibold text-on-surface">{children}</h3>;
}

/** One reason and the lines of evidence behind it. */
function ReasonBlock({ reason, lead }: { reason: CoachAttentionReason; lead: boolean }) {
  const { t, language } = useTranslation();
  return (
    <div className="space-y-3" data-testid={`coach-reason-${reason.id}`}>
      <p className={lead ? 'text-base text-on-surface' : 'text-sm font-medium text-on-surface-variant'}>
        {describeLine(t, language, reason.key, reason.params)}
      </p>
      <ul className="m-0 flex list-none flex-col p-0">
        {reason.evidence.map((line, index) => (
          <li
            key={`${line.key}-${index}`}
            className="flex flex-wrap items-baseline gap-x-4 gap-y-1 border-t ghost-border py-2.5"
            data-testid="coach-evidence"
          >
            <span className="min-w-[240px] flex-1 text-sm text-on-surface">
              {describeLine(t, language, line.key, line.params)}
            </span>
            <span className="font-mono text-xs text-on-surface-variant" data-testid="coach-evidence-source">
              {describeSource(t, language, line.source, line.at)}
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}

function FormBlock({ athlete }: { athlete: CoachRosterAthlete }) {
  const { t, language } = useTranslation();
  const training = athlete.training;
  const band = training?.form?.band ?? 'insufficient_history';
  const cut = trainingCut(training);
  let note: string;
  if (band === 'insufficient_history') note = t('coachHome.detail.formNoteInsufficient');
  else if (cut && training?.form) note = t('coachHome.detail.formNoteStale', { date: civilShortDate(training.form.as_of, language) });
  else if (training?.ctl !== null && training?.ctl !== undefined) note = t('coachHome.detail.formNote', { ctl: training.ctl });
  else note = t('coachHome.detail.formNoteNoCtl');
  return (
    <div className="flex min-w-0 flex-col gap-2.5" data-testid="coach-form-block">
      <div className="flex items-baseline gap-2">
        <span className="text-sm font-semibold text-on-surface">{t('coachHome.detail.form')}</span>
        <span className="text-sm text-on-surface" data-testid="coach-form-word">
          {t(FORM_BAND_LABEL_KEY[band])}
        </span>
      </div>
      <FormStrip band={band} muted={cut} />
      <div aria-hidden="true" className="flex justify-between text-xs text-on-surface-variant">
        <span>{t(FORM_BAND_LABEL_KEY.deep_fatigue)}</span>
        <span>{t(FORM_BAND_LABEL_KEY.detraining)}</span>
      </div>
      <span className="text-xs text-on-surface-variant">{note}</span>
    </div>
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
      <div className="flex min-w-0 flex-col gap-2.5" data-testid="coach-plan-block">
        <span className="text-sm font-semibold text-on-surface">{t('coachHome.detail.plan')}</span>
        <span className="text-sm text-on-surface-variant">
          {draft ? t('coachHome.detail.planDraftOnly') : t('coachHome.detail.planNone')}
        </span>
      </div>
    );
  }

  const first = civilDayNumber(plan.phases[0].start);
  const lastPhase = plan.phases[plan.phases.length - 1];
  const last = civilDayNumber(lastPhase.end ?? lastPhase.start);
  const todayAt = civilDayNumber(today);
  const span = last - first;
  const todayPct = span > 0 ? Math.min(100, Math.max(0, ((todayAt - first) / span) * 100)) : 0;
  const note: string[] = [];
  if (plan.week !== null) note.push(t('coachHome.detail.planWeek', { week: plan.week, total: plan.weeks_total }));
  if (plan.pushed_to.length > 0) {
    note.push(t('coachHome.detail.pushedTo', { platforms: plan.pushed_to.map(providerName).join(', ') }));
  }

  return (
    <div className="flex min-w-0 flex-col gap-2.5" data-testid="coach-plan-block">
      <div className="flex flex-wrap items-baseline gap-2">
        <span className="text-sm font-semibold text-on-surface">{t('coachHome.detail.plan')}</span>
        <span className="text-sm text-on-surface">
          {season?.state === 'a_race_ahead' && race !== undefined
            ? `${race.name} · ${civilShortDate(race.date, language)}`
            : t('coachHome.detail.noARace')}
        </span>
        {season?.state === 'a_race_ahead' && toRace !== null && toRace >= 0 && (
          <span className="font-mono text-xs text-on-surface-variant">{t('coachHome.detail.countdown', { days: toRace })}</span>
        )}
      </div>
      <div className="relative flex gap-[3px]" role="img" aria-label={plan.phase ? t(COACH_PHASE_LABEL_KEY[plan.phase]) : t('coachHome.detail.plan')}>
        {plan.phases.map((phase, index) => {
          const active = phase.kind === plan.phase;
          return (
            <PhaseBand
              key={`${phase.kind}-${index}`}
              style={{ flex: `${Math.max(1, phase.weeks)} 1 0` }}
              className={clsx(
                'h-[22px] min-w-0 rounded-[5px] text-xs font-medium',
                active ? 'bg-primary-container text-on-primary-container' : phaseClass(phase.kind),
              )}
              label={t(COACH_PHASE_LABEL_KEY[phase.kind])}
            />
          );
        })}
        <span
          aria-hidden="true"
          data-testid="coach-today-line"
          style={{ left: `${todayPct}%` }}
          className="absolute -bottom-1 -top-1 w-0.5 rounded-sm bg-on-surface"
        />
      </div>
      {note.length > 0 && <span className="text-xs text-on-surface-variant">{note.join(' · ')}</span>}
    </div>
  );
}

function SourcesBlock({ athlete }: { athlete: CoachRosterAthlete }) {
  const { t, language } = useTranslation();
  const chips = sourceChips(athlete);
  return (
    <div className="flex min-w-0 flex-col gap-2" data-testid="coach-sources-block">
      <span className="text-sm font-semibold text-on-surface">{t('coachHome.detail.sources')}</span>
      {chips.length === 0 && <span className="text-sm text-on-surface-variant">{t('coachHome.detail.sourceNone')}</span>}
      {chips.map((chip) => (
        <div key={chip.provider} className="flex min-h-7 items-center gap-2.5" data-testid={`coach-source-${chip.provider}`}>
          <StatusDot tone={chip.state === 'ok' ? 'ok' : 'bad'} />
          <span className="text-sm font-medium text-on-surface">{providerName(chip.provider)}</span>
          <span className={clsx('ml-auto text-xs', chip.state === 'ok' ? 'text-on-surface-variant' : 'font-medium text-on-surface')}>
            {sourceChipWords(t, language, chip)}
          </span>
        </div>
      ))}
    </div>
  );
}

function AgentBlock({ athlete }: { athlete: CoachRosterAthlete }) {
  const { t, language } = useTranslation();
  const replies = athlete.agent_replies;
  const has = replies !== undefined && replies.count > 0;
  const latest = useQuery({
    queryKey: [...QUERY_KEYS.coachReplies.list(athlete.user_id), 'latest'],
    enabled: has,
    staleTime: 60_000,
    queryFn: () => coachRepliesApi.list({ athlete_id: athlete.user_id, limit: 1 }),
  });
  const reply = latest.data?.replies[0];
  // The chip names what the checker found, in the chat chip's own words: the
  // figures with no source in the athlete's data, then each claim status the
  // reply carries (« 1 chiffre non vérifié · 1 contredite · 2 appuyées »).
  // It warns only on what the shared verdict rule says alerts.
  const chip = useMemo(() => {
    if (reply === undefined) return null;
    const summary = summarizeVerdicts(reply.verdicts);
    const parts = [
      reply.unmatched_figures > 0
        ? t(countKey(reply.unmatched_figures, language, COACH_UNVERIFIED_KEYS), { count: reply.unmatched_figures })
        : '',
      summary === null ? '' : verdictChipLabel(t, summary, language),
    ].filter((part) => part.length > 0);
    const alerts =
      reply.unmatched_figures > 0 || reply.verdicts.some((v) => verdictToneAlerts(VERDICT_STATUS_TONE[v.status]));
    return {
      text: parts.length > 0 ? parts.join(VERDICT_CHIP_SEPARATOR) : t('coachHome.detail.verdictVerified'),
      alerts,
    };
  }, [reply, t, language]);

  return (
    <div className="flex min-w-0 flex-col gap-2" data-testid="coach-agent-block">
      <span className="text-sm font-semibold text-on-surface">{t('coachHome.detail.agent')}</span>
      {!has && <p className="m-0 text-sm text-on-surface-variant">{t('coachHome.detail.agentNone')}</p>}
      {has && (
        <>
          <p className="m-0 text-xs text-on-surface-variant">
            {t(countKey(replies.count, language, COACH_AGENT_COUNT_KEYS), { count: replies.count, verified: replies.verified })}
            {replies.capped ? ` · ${t('coachHome.detail.agentCapped')}` : ''}
          </p>
          {reply !== undefined && reply.withheld && (
            <p className="m-0 text-sm text-on-surface-variant">{t('coachHome.detail.replyWithheld')}</p>
          )}
          {reply !== undefined && !reply.withheld && reply.content !== null && (
            <>
              <p className="m-0 line-clamp-4 text-sm leading-normal text-on-surface" data-testid="coach-agent-reply">
                {reply.content}
              </p>
              <div className="flex flex-wrap items-center gap-2">
                <span
                  data-testid="coach-agent-verdict"
                  className="inline-flex h-[22px] items-center gap-1.5 rounded-full bg-surface-container-high px-2 text-xs font-medium text-on-surface"
                >
                  <StatusDot tone={chip?.alerts === true ? 'warn' : 'ok'} />
                  {chip?.text}
                </span>
                <span className="font-mono text-xs text-on-surface-variant">{formatSyncTime(reply.created_at, language)}</span>
              </div>
            </>
          )}
        </>
      )}
    </div>
  );
}

/**
 * The note under a non-sharer: true of why they do not share. The server
 * raises a revoke only with its date, so the row's own `sharing_stopped_at`
 * decides, as it does on the team table's status line.
 */
function hiddenNoteKey(athlete: CoachRosterAthlete): string {
  if (athlete.status === 'no_source') return 'coachHome.detail.hiddenNoSource';
  return athlete.sharing_stopped_at !== undefined ? 'coachHome.detail.hidden' : 'coachHome.detail.hiddenNever';
}

/** The pane beside À traiter: one athlete, why they are here, and what to do about it. */
export function CoachAthletePane({ athlete, entry, treatedReason, today, destinations, onTreat, onUndo }: CoachAthletePaneProps) {
  const { t, language } = useTranslation();
  const sharing = athlete.status === 'shared' && athlete.training !== undefined;
  const reason = entry?.reason ?? null;
  const primaryAction = reason?.action ?? null;
  const showGroupAction =
    athlete.groups.length > 0 && (primaryAction === null || !destinations.primaryIsGroupRoom(primaryAction));
  const planThreadBusy = destinations.pendingAthleteId === athlete.user_id;

  return (
    <div className="mx-auto flex max-w-[760px] flex-col gap-8" data-testid="coach-athlete-pane">
      <div className="flex flex-wrap items-center gap-x-4 gap-y-3">
        <AthleteAvatar userId={athlete.user_id} name={athlete.name} size="lg" muted={!sharing} />
        <div className="flex min-w-0 flex-col gap-0.5">
          <h2 className="m-0 font-display text-xl font-semibold tracking-tight text-on-surface">{athlete.name}</h2>
          <span className="text-xs text-on-surface-variant">{athlete.groups.map((g) => g.name).join(' · ')}</span>
        </div>
        <div className="flex flex-wrap gap-1.5 md:ml-auto">
          {reason !== null && primaryAction !== null && (
            <Button
              size="sm"
              data-testid="coach-primary-action"
              disabled={planThreadBusy}
              onClick={() => destinations.perform(primaryAction, athlete, reason)}
            >
              {t(COACH_ACTION_LABEL_KEY[primaryAction])}
            </Button>
          )}
          {showGroupAction && (
            <Button
              size="sm"
              variant="secondary"
              data-testid="coach-write-group"
              onClick={() => destinations.writeToGroup(athlete, reason)}
            >
              {t('coachHome.action.writeToGroup')}
            </Button>
          )}
          {reason !== null && (
            <Button size="sm" variant="tertiary" data-testid="coach-treat" onClick={() => onTreat(athlete, reason)}>
              <Check className="mr-1.5 h-4 w-4" aria-hidden="true" />
              {t('coachHome.action.treated')}
            </Button>
          )}
          {treatedReason !== null && (
            <Button size="sm" variant="tertiary" data-testid="coach-undo" onClick={() => onUndo(athlete, treatedReason)}>
              {t('coachHome.action.undo')}
            </Button>
          )}
        </div>
      </div>

      {entry !== null && (
        <section className="flex flex-col gap-3" aria-labelledby="coach-why">
          <h3 id="coach-why" className="font-sans text-sm font-semibold text-on-surface">
            {t('coachHome.detail.why')}
          </h3>
          <ReasonBlock reason={entry.reason} lead />
          {entry.others.map((other) => (
            <ReasonBlock key={other.id} reason={other} lead={false} />
          ))}
        </section>
      )}

      {entry === null && treatedReason !== null && (
        <section className="flex flex-col gap-3" data-testid="coach-treated-note">
          <Heading>{t('coachHome.detail.why')}</Heading>
          <ReasonBlock reason={treatedReason} lead />
          <p className="m-0 text-xs text-on-surface-variant">
            {t('coachHome.detail.treatedNote')}
            {treatedReason.dismissed_at !== null ? ` · ${formatSyncTime(treatedReason.dismissed_at, language)}` : ''}
          </p>
        </section>
      )}

      {entry === null && treatedReason === null && sharing && (
        <EmptyState data-testid="coach-calm">{t('coachHome.detail.calm')}</EmptyState>
      )}

      {sharing && (
        <div className="grid grid-cols-1 gap-x-10 gap-y-8 md:grid-cols-2">
          <FormBlock athlete={athlete} />
          <PlanBlock athlete={athlete} today={today} />
          <SourcesBlock athlete={athlete} />
          <AgentBlock athlete={athlete} />
        </div>
      )}

      {!sharing && (
        <p className="m-0 max-w-[520px] text-sm text-on-surface-variant" data-testid="coach-hidden-note">
          {t(hiddenNoteKey(athlete))}
        </p>
      )}
    </div>
  );
}
