// ABOUTME: Post-connect onboarding step "Your team" — analyzing your data, then the lead and the members to start with
// ABOUTME: Renders between OnboardingConnectProvider and Home; the picker is TeamPicker, the write is the coach_proposal step

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useQueryClient } from '@tanstack/react-query';
import { useTranslation } from '@pierre/i18n';
import { QUERY_KEYS } from '../constants/queryKeys';
import type { TeamStepChoice } from '../hooks/useOnboardingState';
import { chatApi, coachesApi } from '../services/api';
import { Button } from './ui';
import OnboardingShell from './OnboardingShell';
import { homeConversationRoute } from './home/homeRoute';
import { TeamPicker, type TeamChoice } from './onboarding/TeamPicker';
import { useTeamProposal } from './onboarding/useTeamProposal';

/**
 * Onboarding's Your team step (carnet#1095).
 *
 * Shown once, right after the athlete connects their first provider and
 * before Home. On mount it calls `GET /api/agents/proposal`, which scans the
 * recent activities, infers a sport profile, re-ranks the agents for a lead
 * and picks the members the team rules fire for. While that runs we show an
 * "analyzing your data" spinner.
 *
 * Start and Skip write the step and wait for it: the server joins every agent
 * to the team, then makes the lead the selected agent. Only then does Start
 * open the lead's topic in the personal thread and the step hand over to
 * Home — a failed write keeps the athlete here to retry.
 */
export default function OnboardingCoachProposal({
  userDisplayName,
  withoutProvider = false,
  onSave,
  onComplete,
}: {
  userDisplayName?: string | null;
  /** The athlete continued without connecting: no copy may claim their data is read. */
  withoutProvider?: boolean;
  /** Write the step; rejects when the team was not saved. */
  onSave: (choice: TeamStepChoice) => Promise<void>;
  /** The step is done: advance to Home. */
  onComplete: () => void;
}) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const { data, isLoading, isError } = useTeamProposal();

  const handleSubmit = async (choice: TeamChoice) => {
    await onSave(choice);
    // The team is now the athlete's: Home's Your team and Discover read it.
    void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.coaches.lists() });
    void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.store.installations() });
    if (choice.status === 'complete') {
      try {
        // The lead surfaces first wherever agents are listed by use.
        await coachesApi.recordUsage(choice.leadAgentId);
      } catch {
        // Non-fatal: the team is saved; the topic below still opens.
      }
      try {
        // Start means start talking to the lead: a topic in the athlete's
        // personal thread bound to it, opened on Home. The dashboard reads
        // the hash when it mounts, so it is set before the step hands over.
        const { conversation } = await chatApi.openPersonalTopic({ agent_id: choice.leadAgentId });
        window.location.hash = `#${homeConversationRoute(conversation.id)}`;
      } catch {
        // Home still opens on the personal thread; its "+" starts the topic.
      }
    }
    onComplete();
  };

  // "Analyzing your data" — shown while the activity scan + LLM re-rank run.
  if (isLoading) {
    return (
      <OnboardingShell>
        <div className="flex flex-col items-center gap-4 py-8">
          <div className="pierre-spinner w-10 h-10 border-on-surface border-t-transparent" />
          <p className="text-sm text-on-surface">
            {t(withoutProvider ? 'onboarding.preparingTeam' : 'onboarding.analyzingTrainingData')}
          </p>
          <p className="text-xs text-on-surface-variant max-w-sm text-center">
            {t(withoutProvider ? 'onboarding.preparingTeamHint' : 'onboarding.readingActivitiesHint')}
          </p>
        </div>
      </OnboardingShell>
    );
  }

  // No proposal, or one with no agent to lead: the step records itself done
  // without an agent (best-effort, as every step without a team is) and the
  // athlete goes on to Home, where Pick your team waits.
  if (isError || !data || !(data.team?.lead ?? data.agents[0])) {
    return (
      <OnboardingShell>
        <div className="flex flex-col items-center gap-4 py-8">
          <p className="text-sm text-on-surface">{t('onboarding.agentSuggestionsFailed')}</p>
          <Button
            variant="primary"
            onClick={() => {
              void onSave({ status: 'complete' }).catch(() => {});
              onComplete();
            }}
          >
            {t('onboarding.continueToDashboard')}
          </Button>
        </div>
      </OnboardingShell>
    );
  }

  return (
    <OnboardingShell
      heading={
        userDisplayName
          ? t('onboarding.team.heading', { name: userDisplayName })
          : t('onboarding.team.headingNoName')
      }
    >
      <div className="mx-auto max-w-[560px]">
        <p className="mt-2 text-center font-serif text-lg italic text-on-surface-variant">
          {t(withoutProvider ? 'onboarding.team.sublineNoProvider' : 'onboarding.team.subline')}
        </p>
        <div className="mt-7">
          <TeamPicker proposal={data} withoutProvider={withoutProvider} onSubmit={handleSubmit} />
        </div>
      </div>
    </OnboardingShell>
  );
}
