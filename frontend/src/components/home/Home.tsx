// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Home's Today — Getting started, Your team, today's plan, the week, the status, the latest activities — beside the conversation
// ABOUTME: A tap on a day drafts its question in that conversation, a first step sends its starter, one on an activity opens its view

import { clsx } from 'clsx';
import { useTranslation } from '@pierre/i18n';
import { planWindow, useVolumeSportFilter } from '@pierre/ui-logic';
import { DEFAULT_HOME_PREFERENCES } from '@pierre/shared-types';
import { Section } from '../ui/Section';
import { EmptyState } from '../ui/EmptyState';
import { useHomePreferences, useTrainingPlan, useTrainingVolume } from '../../hooks/useHome';
import { HomeToday } from './HomeToday';
import { HomeWeek } from './HomeWeek';
import { HomeStatus } from './HomeStatus';
import { HomeVolume } from './HomeVolume';
import { RecentActivities } from './RecentActivities';
import { GettingStarted } from './GettingStarted';
import { YourTeam } from './YourTeam';
import type { TodayActions } from './personalToday';

interface HomeBriefingProps {
  /** Dashboard route navigator, `tab[/subview]`. */
  onNavigate: (route: string) => void;
  /** What Today's sections do to the conversation beside them: draft, send, open a topic. */
  actions: TodayActions;
  /** Open the team picker over the page, for an athlete with no team. */
  onPickTeam: () => void;
  /** Laid out for the docked side panel: the latest activity's map in its shorter frame. */
  compact?: boolean;
}

/**
 * The plan half of the page. No plan is answered with a quiet link that
 * drafts the request for one, beside a control that sets the suggestion
 * aside — not every athlete wants a plan, and the page's loudest button must
 * not be one they cannot get rid of (carnet#820). The choice is stored on the
 * server, so the phone honours it too, and Settings brings the suggestion
 * back. A plan that could not be read is said as such and never shown as
 * "no plan", because offering to build a plan to an athlete who has one is
 * the wrong answer.
 */
function HomePlan({ onOpenChatDraft }: { onOpenChatDraft: (text: string) => void }) {
  const { t } = useTranslation();
  const plan = useTrainingPlan();
  const home = useHomePreferences();

  // A failed re-read keeps the plan already on screen; only a page with no
  // answer at all says the plan could not be loaded.
  if (plan.data === undefined) {
    return (
      <Section title={t('chat.dayToday')} headingLevel={3} data-testid="home-today">
        {plan.isError ? (
          <EmptyState
            data-testid="home-plan-failed"
            action={{ label: t('common.retry'), onClick: () => void plan.refetch() }}
          >
            {t('home.plan.loadFailed')}
          </EmptyState>
        ) : (
          <div className="flex py-3" role="status" aria-label={t('common.loading')}>
            <div className="pierre-spinner" />
          </div>
        )}
      </Section>
    );
  }

  if (plan.data.plan === null) {
    // The links wait until the choice is known, so a dismissed suggestion
    // never flashes back on load; a choice that could not be read offers it.
    // Set aside, the section keeps its one plain sentence.
    const preferences = home.preferences ?? (home.isError ? DEFAULT_HOME_PREFERENCES : null);
    return (
      <Section title={t('chat.dayToday')} headingLevel={3} data-testid="home-today">
        <div data-testid="home-plan-empty">
          <p className="text-sm font-medium text-on-surface">{t('home.plan.emptyTitle')}</p>
          {preferences !== null && !preferences.plan_suggestion_hidden && (
            <>
              <p className="mt-1 max-w-[560px] text-sm text-on-surface-variant">{t('home.plan.emptyBody')}</p>
              <div className="mt-2 flex flex-wrap items-center gap-x-4 gap-y-1 text-sm">
                <button
                  type="button"
                  data-testid="home-plan-build"
                  className="rounded font-medium text-primary hover:underline focus-ring"
                  onClick={() => onOpenChatDraft(t('home.plan.buildDraft'))}
                >
                  {t('home.plan.buildCta')}
                </button>
                <button
                  type="button"
                  data-testid="home-plan-hide"
                  className="rounded text-on-surface-variant hover:text-on-surface hover:underline focus-ring"
                  aria-label={t('home.plan.hideSuggestionAria')}
                  onClick={() => home.update({ plan_suggestion_hidden: true })}
                >
                  {t('home.plan.hideSuggestion')}
                </button>
              </div>
            </>
          )}
        </div>
      </Section>
    );
  }

  const calendar = planWindow(plan.data.today);
  if (calendar === null) {
    return (
      <Section title={t('chat.dayToday')} headingLevel={3} data-testid="home-today">
        <EmptyState data-testid="home-plan-failed">{t('home.plan.loadFailed')}</EmptyState>
      </Section>
    );
  }

  return <HomeToday plan={plan.data.plan} calendar={calendar} onOpenChatDraft={onOpenChatDraft} />;
}

/**
 * Home's own sections. Getting started and Your team lead (carnet#1098), so
 * a new athlete meets what to do next and who they work with first, then the
 * sections the page always had. The personal surface draws them in its Today
 * panel beside the conversation on a wide screen, and in a drawer or a sheet
 * on a narrower one — which therefore opens on the same two sections. The
 * sport picked on the volume card filters the activities under it.
 */
export function HomeBriefing({ onNavigate, actions, onPickTeam, compact = false }: HomeBriefingProps) {
  const onOpenChatDraft = actions.draft;
  const sportFilter = useVolumeSportFilter(useTrainingVolume().data?.weeks);
  return (
    <div data-testid="home-briefing" className={clsx('space-y-8 px-4', compact ? 'py-2' : 'py-4')}>
      <GettingStarted actions={actions} onNavigate={onNavigate} />
      <YourTeam actions={actions} onNavigate={onNavigate} onPickTeam={onPickTeam} />
      <HomePlan onOpenChatDraft={onOpenChatDraft} />
      <HomeWeek onOpenChatDraft={onOpenChatDraft} onNavigate={onNavigate} />
      <HomeStatus />
      <HomeVolume filter={sportFilter} />
      <RecentActivities onNavigate={onNavigate} compact={compact} sport={sportFilter.sport} />
    </div>
  );
}
