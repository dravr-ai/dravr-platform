// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A coach's workspace on one athlete (direction B) — the plan's two weeks to edit, Dravr's proposals to accept or refuse, publish, and one panel of threads
// ABOUTME: Reads the plan thread's subject plan and writes through the day editor's hooks; the panel beside it from lg, a sheet below (carnet#1816)

import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { ArrowLeft } from 'lucide-react';
import { clsx } from 'clsx';
import { useTranslation, type TFunction } from '@pierre/i18n';
import type { CoachRosterAthlete, DraftState, PlanSubject, WorkoutPlan } from '@pierre/shared-types';
import { PLAN_DRAFT_KEYS, PLAN_THREAD_KEYS, countKey, prePublishChecks } from '@pierre/chat-utils';
import {
  PLAN_WORKSPACE_KEYS,
  formatCivilDate,
  openThreadRefusalText,
  planCalendarWeeks,
  workspaceAgentName,
  workspaceGroupTab,
  workspacePlanState,
} from '@pierre/ui-logic';
import { useAuth } from '../../../hooks/useAuth';
import { useIsDesktop, useIsMobile } from '../../../hooks/useBreakpoint';
import { usePlanDraftCard, useSubjectPlan } from '../../../hooks/useCoachPlan';
import { useWorkspaceAnswers, useWorkspaceThreads } from '../../../hooks/useAthleteWorkspace';
import { Button, EmptyState, IconButton, Sheet } from '../../ui';
import { PlanDraftActions } from '../../chat/PlanDraftControls';
import type { SessionEditorOpening } from '../../chat/SessionEditorSheet';
import type { PendingComposerAction, WorkspacePlanSurface } from '../../chat/types';
import { AthleteAvatar } from '../parts';
import { PhaseBand } from '../PhaseBand';
import { phaseClass } from '../phaseStyle';
import { coachHomeRoute } from '../coachHomeRoute';
import type { CoachDestinations } from '../useCoachDestinations';
import { PrePublishCheck } from './PrePublishCheck';
import { WorkspaceCalendar } from './WorkspaceCalendar';
import { WorkspacePanel, WorkspacePanelOpeners, type WorkspaceTab } from './WorkspacePanel';

// The editor loads when a coach first opens it, never before first paint, as
// on the plan card: it is a coach-only sheet.
const SessionEditorSheet = lazy(() => import('../../chat/SessionEditorSheet'));

/** The goal race's date with its year: « 30 avr. 2027 ». */
const RACE_DATE: Intl.DateTimeFormatOptions = { day: 'numeric', month: 'short', year: 'numeric' };

interface AthleteWorkspaceProps {
  athlete: CoachRosterAthlete;
  /** Dashboard route navigator, `tab[/subview]`. */
  onNavigate: (route: string) => void;
  /** Where Coach Home's actions go: « Écrire dans le groupe » for an athlete with no room of their own. */
  destinations: CoachDestinations;
}

/**
 * The header's publish control: « Publier N changements », N the dates the
 * draft changes, through the draft card's own hook and view, so the conflict,
 * Publish anyway and Discard are the ones every plan card has. With no draft
 * open it stands disabled. Keyed by the draft by its caller, so a new draft
 * starts from no outcome.
 */
function WorkspacePublish({
  conversationId,
  subject,
  draft,
  count,
}: {
  conversationId: string;
  subject: PlanSubject;
  draft: DraftState | null;
  count: number;
}) {
  const { t, language } = useTranslation();
  const { user } = useAuth();
  const card = usePlanDraftCard(conversationId, user?.id ?? null, draft);
  const label = t(countKey(count, language, PLAN_WORKSPACE_KEYS.publish), { count });
  if (card.state.kind === 'hidden') {
    return (
      <Button size="sm" disabled data-testid="workspace-publish-idle">
        {label}
      </Button>
    );
  }
  return (
    <PlanDraftActions
      conversationId={conversationId}
      subject={subject}
      card={card}
      layout="bar"
      publishLabel={label}
      canPublish={count > 0}
    />
  );
}

/** A phase as its band and the caption name it: « Préparation · 2 sem ». */
function phaseName(phase: WorkoutPlan['phases'][number], t: TFunction): string {
  return `${t(`plan.card.phase.${phase.kind}`)} · ${t('plan.card.weeksShort', { count: phase.weeks })}`;
}

/**
 * The season: one band per phase, as long as its weeks, the current one lit,
 * and a caption naming the current one, since a short phase's band is too
 * narrow to carry its own name.
 */
function SeasonBand({ plan }: { plan: WorkoutPlan }) {
  const { t } = useTranslation();
  if (plan.phases.length === 0) return null;
  const current = plan.phases.find((phase) => phase.current);
  return (
    <div className="flex flex-col gap-1.5" data-testid="workspace-season">
      <div
        className="flex gap-[3px]"
        role="img"
        aria-label={current ? phaseName(current, t) : t('plan.card.season')}
      >
        {plan.phases.map((phase) => (
          <PhaseBand
            key={`${phase.kind}-${phase.start}`}
            style={{ flex: `${Math.max(1, phase.weeks)} 1 0` }}
            className={clsx(
              'h-[22px] min-w-0 rounded-[5px] text-xs font-medium',
              phase.current ? 'bg-primary-container text-on-primary-container ring-1 ring-primary' : phaseClass(phase.kind),
            )}
            label={phaseName(phase, t)}
            data-kind={phase.kind}
          />
        ))}
      </div>
      {current && (
        <p aria-hidden="true" className="m-0 text-xs text-on-surface-variant">
          <span className="font-medium text-on-surface">{phaseName(current, t)}</span>
          {' · '}
          {t('plan.card.now')}
        </p>
      )}
    </div>
  );
}

/**
 * One athlete's plan as the coach works it: the header with the goal race
 * and publish, the pre-publish check, the season and the two weeks, and the
 * panel of Historique, the private thread with the agent and the athlete's
 * room. The plan is the plan thread's subject-plan read: the draft when one
 * is open, else the live plan; a date the read names proposed — one the draft
 * changes over the live plan — is a proposal the coach accepts, for this
 * session only (decision W1), or refuses, which puts the live plan back on it.
 */
export default function AthleteWorkspace({ athlete, onNavigate, destinations }: AthleteWorkspaceProps) {
  const { t, language } = useTranslation();
  const isDesktop = useIsDesktop();
  const isMobile = useIsMobile();
  const threads = useWorkspaceThreads(athlete);
  const { planThreadId } = threads;
  const plan = useSubjectPlan(planThreadId, planThreadId !== null);
  const subject = plan.read?.status === 'ok' ? plan.read.plan : null;
  const name = subject?.athlete.display_name.trim() || athlete.name;

  const shown = subject ? (subject.draft ?? subject.live) : null;
  // Which dates are proposed is the server's to say, by the rule a publish
  // announces them with: it names none when no draft is open.
  const proposed = useMemo(() => subject?.proposed_dates ?? [], [subject]);
  const weeks = useMemo(
    () => (shown && subject ? planCalendarWeeks(shown, proposed, subject.today) : []),
    [shown, subject, proposed],
  );
  const checks = useMemo(
    () => (subject?.draft ? prePublishChecks(subject.draft, subject.today) : []),
    [subject],
  );

  // The draft the publish control acts on, held past its publish so the
  // control can say it landed; a newer draft replaces it.
  const draftState = subject?.draft_state ?? null;
  const [heldDraft, setHeldDraft] = useState<DraftState | null>(null);
  useEffect(() => {
    if (draftState !== null) setHeldDraft(draftState);
  }, [draftState]);
  const publishDraft = draftState ?? heldDraft;

  const answers = useWorkspaceAnswers(planThreadId);

  const [editing, setEditing] = useState<SessionEditorOpening | null>(null);
  const [tab, setTab] = useState<WorkspaceTab>('history');
  const [panelOpen, setPanelOpen] = useState(false);
  const [composerAction, setComposerAction] = useState<PendingComposerAction | null>(null);

  // « Voir sur le calendrier » from the agent's thread: the sheet steps aside
  // below lg, and the calendar comes into view and takes focus.
  const calendarRef = useRef<HTMLElement>(null);
  const showCalendar = useCallback(() => {
    setPanelOpen(false);
    const calendar = calendarRef.current;
    if (calendar === null) return;
    calendar.scrollIntoView?.({ block: 'start' });
    calendar.focus({ preventScroll: true });
  }, []);
  const planSurface = useMemo<WorkspacePlanSurface>(() => ({ kind: 'workspace', onShowCalendar: showCalendar }), [showCalendar]);

  const openTab = (next: WorkspaceTab) => {
    setTab(next);
    setPanelOpen(true);
  };

  // « Rédiger le plan de … » with no plan yet: the agent's tab takes the
  // starter, as the plan thread's own one-tap start sends it.
  const draftPlan = () => {
    setComposerAction({ kind: 'send', text: t(PLAN_THREAD_KEYS.starterMessage, { name }) });
    openTab('dravr');
  };

  const agentName = workspaceAgentName(threads.planGroup, t('shell.brandName'));
  const groupTab = workspaceGroupTab(threads, athlete);

  const goal = shown?.goal_race ?? null;

  // Which state the plan is in is decided once, for both clients; this only draws it.
  const planState = workspacePlanState(athlete, threads, plan);
  let calendar: React.ReactNode;
  switch (planState.kind) {
    case 'noGroup':
      calendar = <EmptyState data-testid="workspace-thread-failed">{t(PLAN_THREAD_KEYS.openError)}</EmptyState>;
      break;
    case 'threadFailed':
      calendar = (
        <EmptyState action={{ label: t('common.retry'), onClick: planState.retry }} data-testid="workspace-thread-failed">
          {openThreadRefusalText(planState.error, name, t)}
        </EmptyState>
      );
      break;
    case 'groupsFailed':
      calendar = (
        <EmptyState action={{ label: t('common.retry'), onClick: planState.retry }} data-testid="workspace-thread-failed">
          {t(PLAN_THREAD_KEYS.openError)}
        </EmptyState>
      );
      break;
    case 'loading':
      calendar = (
        <div className="flex justify-center py-12" role="status" aria-label={t('common.loading')}>
          <div className="pierre-spinner" />
        </div>
      );
      break;
    case 'consentRevoked':
      calendar = <EmptyState data-testid="workspace-consent-revoked">{t(PLAN_DRAFT_KEYS.consentRevoked, { name })}</EmptyState>;
      break;
    case 'loadFailed':
      calendar = (
        <EmptyState action={{ label: t('common.retry'), onClick: planState.retry }} data-testid="workspace-plan-failed">
          {t(PLAN_WORKSPACE_KEYS.loadFailed, { name })}
        </EmptyState>
      );
      break;
    case 'noPlan':
      calendar = (
        <EmptyState
          action={{ label: t(PLAN_THREAD_KEYS.starterLabel, { name }), onClick: draftPlan, 'data-testid': 'workspace-draft-plan' }}
          data-testid="workspace-no-plan"
        >
          {t(PLAN_WORKSPACE_KEYS.noPlan, { name })}
        </EmptyState>
      );
      break;
    case 'plan':
      calendar = (
        <div className="flex flex-col gap-5">
          <PrePublishCheck
            items={checks}
            athleteName={name}
            onKeep={(item) => answers.setCall(item.day, item.position, true)}
            keeping={answers.callSaving}
            onReview={(item) => setEditing({ day: item.day, entries: item.entries, mode: 'edit' })}
            onCleared={() => calendarRef.current?.focus()}
          />
          <SeasonBand plan={planState.shown} />
          <WorkspaceCalendar
            weeks={weeks}
            isAccepted={answers.isAccepted}
            onAccept={answers.accept}
            onRefuse={answers.refuse}
            refusing={answers.refusing}
            onEdit={setEditing}
          />
        </div>
      );
      break;
  }

  const panel = (
    <WorkspacePanel
      tab={tab}
      onTab={setTab}
      athlete={athlete}
      agentName={agentName}
      planThreadId={planThreadId}
      group={groupTab}
      onOpenRoom={threads.openRoom}
      roomOpening={threads.roomOpening}
      roomError={threads.roomError}
      onWriteToGroup={() => destinations.writeToGroup(athlete, null)}
      planSurface={planSurface}
      composerAction={composerAction}
      onComposerActionConsumed={() => setComposerAction(null)}
    />
  );

  return (
    <div className="flex h-full min-h-0 flex-col bg-surface" data-testid="athlete-workspace">
      <header className="flex min-h-[52px] shrink-0 flex-wrap items-center gap-x-3 gap-y-2 border-b ghost-border px-4 py-2 lg:px-6">
        <IconButton
          aria-label={t('common.back')}
          onClick={() => onNavigate(coachHomeRoute({ view: 'queue', athleteId: athlete.user_id }))}
          data-testid="workspace-back"
        >
          <ArrowLeft className="h-4 w-4" aria-hidden="true" />
        </IconButton>
        <AthleteAvatar userId={athlete.user_id} name={name} />
        <div className="flex min-w-0 flex-1 flex-col">
          <h1 className="m-0 truncate font-display text-lg font-semibold tracking-tight text-on-surface">{name}</h1>
          <p className="m-0 truncate text-xs text-on-surface-variant" data-testid="workspace-goal">
            {goal !== null && (
              <>
                {t('plan.card.goalRace')} <span className="font-medium text-on-surface">{goal.name}</span>
                {' · '}
                <span className="font-mono">{formatCivilDate(goal.date, language, RACE_DATE)}</span>
                {athlete.groups.length > 0 && ' · '}
              </>
            )}
            {athlete.groups.map((group) => group.name).join(' · ')}
          </p>
        </div>
        {planThreadId !== null && subject !== null && (
          <WorkspacePublish
            key={publishDraft?.draft_id ?? 'none'}
            conversationId={planThreadId}
            subject={subject.athlete}
            draft={publishDraft}
            count={proposed.length}
          />
        )}
      </header>

      {!isDesktop && (
        // Below lg the panel is a sheet: each tab opens it on itself.
        <WorkspacePanelOpeners agentName={agentName} hasGroup={groupTab !== null} onOpen={openTab} />
      )}

      <div className="flex min-h-0 flex-1">
        <section
          ref={calendarRef}
          tabIndex={-1}
          aria-labelledby="athlete-workspace-calendar"
          // `relative`: the dates' screen-reader-only labels are positioned,
          // and without a containing block here they resolve past the
          // scroller and stretch the page to the calendar's full height.
          className="relative min-w-0 flex-1 overflow-y-auto px-4 py-4 focus:outline-none lg:px-6"
          data-testid="workspace-calendar-section"
        >
          <h2 id="athlete-workspace-calendar" className="sr-only">
            {t(PLAN_WORKSPACE_KEYS.calendar)}
          </h2>
          {calendar}
        </section>
        {isDesktop && (
          <aside
            aria-label={t(PLAN_WORKSPACE_KEYS.panel)}
            className="flex w-[320px] shrink-0 flex-col border-l ghost-border xl:w-[360px]"
            data-testid="workspace-aside"
          >
            {panel}
          </aside>
        )}
      </div>

      {!isDesktop && panelOpen && (
        <Sheet
          side={isMobile ? 'bottom' : 'right'}
          width="lg"
          title={t(PLAN_WORKSPACE_KEYS.panel)}
          onClose={() => setPanelOpen(false)}
          data-testid="workspace-panel-sheet"
        >
          {panel}
        </Sheet>
      )}

      {editing !== null && planThreadId !== null && subject !== null && (
        <Suspense fallback={null}>
          <SessionEditorSheet
            conversationId={planThreadId}
            subject={subject.athlete}
            opening={editing}
            onClose={() => setEditing(null)}
          />
        </Suspense>
      )}
    </div>
  );
}
