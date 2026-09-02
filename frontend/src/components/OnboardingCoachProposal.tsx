// ABOUTME: Post-connect onboarding step — "analyzing your data" then an inferred profile + top-3 coach proposal
// ABOUTME: Renders between OnboardingConnectProvider and the dashboard; backed by GET /api/coaches/proposal

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useQuery } from '@tanstack/react-query';
import { useState } from 'react';
import type { ProposedCoach } from '@pierre/shared-types';
import { activitySportLabelKey, coachCategoryLabelKey } from '@pierre/shared-constants';
import { defaultConversationTitle } from '@pierre/chat-utils';
import { chatApi, coachesApi } from '../services/api';
import { Button } from './ui';
import OnboardingShell from './OnboardingShell';
import { useTranslation } from '@pierre/i18n';

/** One-time onboarding snapshot; not invalidated elsewhere. */
const COACH_PROPOSAL_QUERY_KEY = ['coaches', 'proposal'] as const;

/**
 * Onboarding coach proposal.
 *
 * Shown once, right after the user connects their first provider and before the
 * dashboard. On mount it calls `GET /api/coaches/proposal`, which scans the
 * user's recent activities, infers a sport profile, and asks the LLM to pick the
 * best ≤3 coaches with a rationale each. While that runs we show an "analyzing
 * your data" spinner — the deliberate pause in the
 * `connect → analyzing → profile → coaches` flow.
 *
 * Completion is persisted in `localStorage` (keyed by user) so the screen is a
 * one-time onboarding step, not a recurring interstitial.
 */
export default function OnboardingCoachProposal({
  userDisplayName,
  onComplete,
}: {
  userDisplayName?: string | null;
  onComplete: () => void;
}) {
  const { t, language } = useTranslation();
  const [selecting, setSelecting] = useState<string | null>(null);

  // A sport as the profile spells it, in the athlete's language when the
  // vocabulary knows it and as spelled on the wire when it does not.
  const sportLabel = (sport: string): string => {
    const key = activitySportLabelKey(sport);
    return key ? t(key) : sport;
  };

  const { data, isLoading, isError } = useQuery({
    queryKey: COACH_PROPOSAL_QUERY_KEY,
    queryFn: () => coachesApi.getProposal(),
    // The proposal reflects a one-time onboarding snapshot; no need to refetch.
    staleTime: Infinity,
    retry: 1,
  });

  const handleStart = async (coachId: string, coachTitle: string) => {
    setSelecting(coachId);
    try {
      // Mark the chosen coach as used so it surfaces first on the dashboard.
      await coachesApi.recordUsage(coachId);
    } catch {
      // Non-fatal: the choice below still opens the coach's thread.
    }
    try {
      // « Démarrer » means start talking to this coach: open a thread bound to
      // it and land inside it. The dashboard reads `#chat/<id>` when it mounts,
      // so the hash is set before onboarding hands over to it.
      const conversation = await chatApi.createConversation({
        coach_id: coachId,
        title: coachTitle || defaultConversationTitle(t('chat.newConversationTitlePrefix'), new Date(), language),
      });
      window.location.hash = `#chat/${encodeURIComponent(conversation.id)}`;
    } catch {
      // The dashboard still opens; the "+" beside the chat starts the thread.
    }
    onComplete();
  };

  // "Analyzing your data" — shown while the activity scan + LLM re-rank run.
  if (isLoading) {
    return (
      <OnboardingShell>
        <div className="flex flex-col items-center gap-4 py-8">
          <div className="pierre-spinner w-10 h-10 border-on-surface border-t-transparent" />
          <p className="text-sm text-on-surface font-label">{t('onboarding.analyzingTrainingData')}</p>
          <p className="text-xs text-on-surface-variant max-w-sm text-center">
            {t('onboarding.readingActivitiesHint')}
          </p>
        </div>
      </OnboardingShell>
    );
  }

  if (isError || !data) {
    return (
      <OnboardingShell>
        <div className="flex flex-col items-center gap-4 py-8">
          <p className="text-sm text-on-surface font-label">
            {t('onboarding.coachSuggestionsFailed')}
          </p>
          <Button variant="primary" onClick={onComplete}>
            {t('onboarding.continueToDashboard')}
          </Button>
        </div>
      </OnboardingShell>
    );
  }

  const { profile, coaches } = data;
  const primary = profile.primary_sport;

  return (
    <OnboardingShell
      heading={
        userDisplayName ? t('app.obStartingLineup', { name: userDisplayName }) : t('frag.startingLineup')
      }
    >
      {/* Inferred profile summary */}
      <div className="mt-6 rounded-xl border border-outline-variant bg-surface-container px-5 py-4">
        {profile.has_profile ? (
          <>
            <p className="text-sm text-on-surface font-label">
              {t('app.obWindowSummary', {
                days: profile.window_days,
                count: profile.total_activities,
              })}
              {primary ? t('app.obMostlySport', { sport: sportLabel(primary) }) : ''}
            </p>
            <div className="mt-3 space-y-1.5">
              {profile.sport_mix.map((s) => (
                <div key={s.sport} className="flex items-center gap-3">
                  <span className="w-20 text-xs text-on-surface-variant">{sportLabel(s.sport)}</span>
                  <div className="flex-1 h-2 rounded-full bg-surface-container-high overflow-hidden">
                    <div
                      className="h-full boreal-hero-gradient"
                      style={{ width: `${Math.round(s.share * 100)}%` }}
                    />
                  </div>
                  <span className="w-10 text-right text-xs text-on-surface-variant">
                    {Math.round(s.share * 100)}%
                  </span>
                </div>
              ))}
            </div>
          </>
        ) : (
          <p className="text-sm text-on-surface-variant font-label">
            {t('onboarding.coachProposalNoActivities')}
          </p>
        )}
      </div>

      {/* Proposed coaches */}
      <div className="mt-6 space-y-3">
        {coaches.map((proposed) => (
          <CoachProposalCard
            key={proposed.coach.id}
            proposed={proposed}
            selecting={selecting === proposed.coach.id}
            disabled={selecting !== null}
            onStart={() => void handleStart(proposed.coach.id, proposed.coach.title)}
          />
        ))}
      </div>

      <div className="mt-8">
        <Button variant="secondary" onClick={onComplete} className="w-full" disabled={selecting !== null}>
          {t('onboarding.skipForNow')}
        </Button>
      </div>
    </OnboardingShell>
  );
}

/** A single proposed-coach card with its match rationale and a start CTA. */
function CoachProposalCard({
  proposed,
  selecting,
  disabled,
  onStart,
}: {
  proposed: ProposedCoach;
  selecting: boolean;
  disabled: boolean;
  onStart: () => void;
}) {
  const { t } = useTranslation();
  const { coach, reason } = proposed;
  return (
    <div className="rounded-xl border border-outline-variant bg-surface-container-low px-5 py-4">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <h3 className="font-display font-semibold text-base text-on-surface truncate">
            {coach.title}
          </h3>
          <p className="mt-0.5 text-xs uppercase tracking-wide text-on-surface-variant">
            {t(coachCategoryLabelKey(coach.category))}
          </p>
        </div>
        <Button variant="primary" onClick={onStart} disabled={disabled}>
          {selecting ? t('onboarding.starting') : t('onboarding.startButton')}
        </Button>
      </div>
      {reason ? <p className="mt-2 text-sm text-on-surface-variant font-label">{reason}</p> : null}
    </div>
  );
}
