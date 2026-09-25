// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The athlete's Home — where sign-in lands and the logo leads: today's plan, the week, the latest activities
// ABOUTME: One reading column under the page header; every tap on a day or an activity opens a chat with the question drafted

import { useTranslation } from '@pierre/i18n';
import { TabHeader } from '../ui/TabHeader';
import { Section } from '../ui/Section';
import { EmptyState } from '../ui/EmptyState';
import { useTrainingPlan } from '../../hooks/useHome';
import { HomeToday } from './HomeToday';
import { HomeWeek } from './HomeWeek';
import { RecentActivities } from './RecentActivities';
import { planWindow } from './homeFormat';

interface HomeProps {
  /** Dashboard route navigator, `tab[/subview]`. */
  onNavigate: (route: string) => void;
  /** Open a new chat whose composer holds `text`, for the athlete to finish and send. */
  onOpenChatDraft: (text: string) => void;
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

export default function Home({ onNavigate, onOpenChatDraft }: HomeProps) {
  const { t } = useTranslation();
  return (
    <div className="flex h-full flex-col" data-testid="home-page">
      <TabHeader title={t('nav.home')} />
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto w-full max-w-[720px] space-y-10 px-4 py-6 md:px-6">
          <HomePlan onOpenChatDraft={onOpenChatDraft} />
          <RecentActivities onNavigate={onNavigate} onOpenChatDraft={onOpenChatDraft} />
        </div>
      </div>
    </div>
  );
}
