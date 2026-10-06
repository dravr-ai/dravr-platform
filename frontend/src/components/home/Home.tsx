// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Home's Today — today's plan, the week, the training status, the latest activities — beside the athlete's conversation
// ABOUTME: A tap on a day drafts its question in that conversation, one on an activity opens its view

import { clsx } from 'clsx';
import { useTranslation } from '@pierre/i18n';
import { Section } from '../ui/Section';
import { EmptyState } from '../ui/EmptyState';
import { useTrainingPlan } from '../../hooks/useHome';
import { HomeToday } from './HomeToday';
import { HomeWeek } from './HomeWeek';
import { HomeStatus } from './HomeStatus';
import { RecentActivities } from './RecentActivities';
import { planWindow } from './homeFormat';

interface HomeBriefingProps {
  /** Dashboard route navigator, `tab[/subview]`. */
  onNavigate: (route: string) => void;
  /** Put `text` in the conversation's composer, for the athlete to finish and send. */
  onOpenChatDraft: (text: string) => void;
  /** Laid out for the docked side panel: the latest activity's map in its shorter frame. */
  compact?: boolean;
}

/**
 * The plan half of the page. No plan is answered with the one filled button
 * on the page, which drafts the request for one; a plan that could not be
 * read is said as such and never shown as "no plan", because offering to
 * build a plan to an athlete who has one is the wrong answer.
 */
function HomePlan({ onOpenChatDraft }: { onOpenChatDraft: (text: string) => void }) {
  const { t } = useTranslation();
  const plan = useTrainingPlan();

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
    return (
      <Section title={t('chat.dayToday')} headingLevel={3} data-testid="home-today">
        <div data-testid="home-plan-empty">
          <p className="text-base font-medium text-on-surface">{t('home.plan.emptyTitle')}</p>
          <p className="mt-1 max-w-[560px] text-sm text-on-surface-variant">{t('home.plan.emptyBody')}</p>
          <button
            type="button"
            className="btn-primary mt-4"
            onClick={() => onOpenChatDraft(t('home.plan.buildDraft'))}
          >
            {t('home.plan.buildCta')}
          </button>
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

  return (
    <>
      <HomeToday plan={plan.data.plan} calendar={calendar} onOpenChatDraft={onOpenChatDraft} />
      <HomeWeek plan={plan.data.plan} calendar={calendar} onOpenChatDraft={onOpenChatDraft} />
    </>
  );
}

/**
 * Home's own sections, in the order the page always had them. The personal
 * surface draws them in its Today panel beside the conversation on a wide
 * screen, and in a drawer or a sheet on a narrower one.
 */
export function HomeBriefing({ onNavigate, onOpenChatDraft, compact = false }: HomeBriefingProps) {
  return (
    <div data-testid="home-briefing" className={clsx('space-y-8 px-4', compact ? 'py-2' : 'py-4')}>
      <HomePlan onOpenChatDraft={onOpenChatDraft} />
      <HomeStatus />
      <RecentActivities onNavigate={onNavigate} compact={compact} />
    </div>
  );
}
