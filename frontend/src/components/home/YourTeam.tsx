// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Today's Your team — the agent leading the athlete's thread, then the other agents on their team, and the way to Discover
// ABOUTME: A row opens a topic with that agent; an athlete with no team is offered the onboarding picker, never a silent backfill (carnet#1098)

import { useMemo, useState } from 'react';
import { clsx } from 'clsx';
import { Plus } from 'lucide-react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from '@pierre/i18n';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { Agent } from '@pierre/shared-types';
import { avatarSlot, initialsFor } from '@pierre/chat-utils';
import { chatApi, coachesApi, userApi } from '../../services/api';
import { usePersonalConversation } from '../../hooks/useConversationList';
import { avatarShapeClass, avatarSlotClass } from '../chat/avatarSlots';
import { Section } from '../ui/Section';
import { EmptyState } from '../ui/EmptyState';
import { Sheet } from '../ui/Sheet';
import { TeamPicker, type TeamChoice } from '../onboarding/TeamPicker';
import { useTeamProposal } from '../onboarding/useTeamProposal';
import { useOnboardingStatus } from '../../hooks/useOnboardingStatus';
import type { TodayActions } from '../ChatTab';

/** The agent list changes when the athlete adds or removes one, which invalidates it; the cache stays warm. */
const AGENT_LIST_STALE_MS = 5 * 60_000;

interface YourTeamProps {
  actions: TodayActions;
  /** Dashboard route navigator, `tab[/subview]`. */
  onNavigate: (route: string) => void;
  /** Open the team picker over the page, for an athlete with no team. */
  onPickTeam: () => void;
}

/**
 * The athlete's team, lead first, then the others by title.
 *
 * The team is `is_assigned` on the agent list — the one membership rule the
 * mention palette and Discover read (carnet#1096). The lead is the agent the
 * personal thread is bound to (carnet#1101); a lead that is not on the team
 * marks nobody.
 */
function orderTeam(agents: readonly Agent[], leadId: string | null): { lead: Agent | null; members: Agent[] } {
  const team = agents.filter((agent) => agent.is_assigned === true);
  const lead = leadId === null ? null : (team.find((agent) => agent.id === leadId) ?? null);
  const members = team
    .filter((agent) => agent !== lead)
    .sort((a, b) => a.title.localeCompare(b.title));
  return { lead, members };
}

function AgentAvatar({ agent }: { agent: Agent }) {
  return (
    <span
      aria-hidden="true"
      className={clsx(
        'flex h-8 w-8 shrink-0 items-center justify-center text-xs font-semibold',
        avatarShapeClass('circle'),
        avatarSlotClass(avatarSlot({ id: agent.id, agent_id: agent.id, group_id: null, subject_user_id: null })),
      )}
    >
      {initialsFor(agent.title)}
    </span>
  );
}

const ROW_CLASS =
  'flex w-full items-center gap-3 rounded-lg px-2 py-1.5 text-left transition-colors hover:bg-surface-container-low focus-ring disabled:opacity-60';

export function YourTeam({ actions, onNavigate, onPickTeam }: YourTeamProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const personal = usePersonalConversation();
  const [openError, setOpenError] = useState(false);
  const agentsQuery = useQuery({
    queryKey: QUERY_KEYS.coaches.list(),
    queryFn: () => coachesApi.list(),
    staleTime: AGENT_LIST_STALE_MS,
  });
  const { lead, members } = useMemo(
    () => orderTeam(agentsQuery.data?.agents ?? [], personal.agentId),
    [agentsQuery.data, personal.agentId],
  );

  // A row opens a fresh topic in the personal thread, bound to that agent —
  // the same path Discover's "Open chat" takes — and lands on it.
  const openTopic = useMutation({
    mutationFn: (agentId: string) => chatApi.openPersonalTopic({ agent_id: agentId }),
    onSuccess: ({ conversation, divider }) => {
      void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() });
      void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.messages(conversation.id) });
      void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.personal() });
      setOpenError(false);
      actions.openTopic(conversation.id, divider?.id ?? null);
    },
    onError: () => setOpenError(true),
  });

  const findMore = (
    <button
      type="button"
      data-testid="home-team-find-more"
      className="rounded text-xs font-medium text-primary hover:underline focus-ring"
      onClick={() => onNavigate('discover')}
    >
      {t('home.team.findMore')}
    </button>
  );

  // Nothing is said until the list answers, so an athlete with a team never
  // sees "No team yet." flash on load; a list that failed says nothing
  // rather than "no team".
  if (agentsQuery.data === undefined || personal.isLoading) {
    return null;
  }

  const team = lead === null ? members : [lead, ...members];

  return (
    <Section title={t('home.team.title')} headingLevel={3} actions={findMore} data-testid="home-team">
      {team.length === 0 ? (
        <EmptyState
          data-testid="home-team-empty"
          action={{ label: t('home.team.pick'), onClick: onPickTeam, 'data-testid': 'home-team-pick' }}
        >
          {t('home.team.empty')}
        </EmptyState>
      ) : (
        <ul className="-mx-2 space-y-0.5">
          {team.map((agent) => {
            const isLead = agent === lead;
            const subline = isLead
              ? t('home.team.leads')
              : [agent.handle ? `@${agent.handle}` : null, agent.description].filter(Boolean).join(' · ');
            return (
              <li key={agent.id}>
                <button
                  type="button"
                  data-testid={isLead ? 'home-team-lead' : 'home-team-member'}
                  aria-label={t('home.team.openAgentAria', { agent: agent.title })}
                  className={ROW_CLASS}
                  disabled={openTopic.isPending}
                  onClick={() => openTopic.mutate(agent.id)}
                >
                  <AgentAvatar agent={agent} />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-sm font-semibold text-on-surface">{agent.title}</span>
                    {subline.length > 0 && (
                      <span className="block truncate text-xs text-on-surface-variant">{subline}</span>
                    )}
                  </span>
                </button>
              </li>
            );
          })}
          <li>
            <button
              type="button"
              data-testid="home-team-add"
              className={ROW_CLASS}
              onClick={() => onNavigate('discover')}
            >
              <span
                aria-hidden="true"
                className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full border border-dashed border-outline text-on-surface-variant"
              >
                <Plus className="h-4 w-4" />
              </span>
              <span className="text-sm font-medium text-primary">{t('home.team.add')}</span>
            </button>
          </li>
        </ul>
      )}
      {openError && (
        <p role="alert" className="mt-2 text-sm text-error">
          {t('app.couldNotOpenChat')}
        </p>
      )}
    </Section>
  );
}

interface PickTeamSheetProps {
  /** A bottom sheet on a phone, a side panel on a wider screen. */
  side: 'bottom' | 'right';
  onClose: () => void;
}

/**
 * Pick your team, over Home, for an athlete with none — everyone onboarded
 * before the Your team step shipped. The onboarding picker, on the same
 * proposal, writing the same `coach_proposal` step: the server joins every
 * agent, then `select_onboarding_lead` picks the lead and moves the thread,
 * so there is one path to a team whichever screen took it.
 */
export function PickTeamSheet({ side, onClose }: PickTeamSheetProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const proposal = useTeamProposal();
  // With no provider connected, no copy may claim the athlete's training is
  // being read.
  const withoutProvider = useOnboardingStatus().data?.needs_provider_connection === true;

  // Picking a team from Home always completes the step, Skip included (it
  // keeps the lead alone): a stored `complete` re-sent from here must never
  // flip to `skipped`, and `onboarding.step_completed` counts the transition.
  const handleSubmit = async (choice: TeamChoice) => {
    await userApi.setOnboardingStep('coach_proposal', 'complete', {
      agentId: choice.leadAgentId,
      teamAgentIds: choice.teamAgentIds,
    });
    void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.coaches.lists() });
    void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.store.installations() });
    // The lead may have moved the thread and posted its welcome there.
    void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.personal() });
    void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() });
    void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.home.gettingStarted() });
    onClose();
  };

  const lead = proposal.data?.team?.lead ?? proposal.data?.agents[0];
  return (
    <Sheet side={side} title={t('home.team.pick')} onClose={onClose} data-testid="home-team-pick-sheet">
      <div className="px-4 py-4">
        {proposal.isLoading ? (
          <div className="flex flex-col items-center gap-3 py-8" role="status">
            <div className="pierre-spinner" />
            <p className="text-sm text-on-surface">
              {t(withoutProvider ? 'onboarding.preparingTeam' : 'onboarding.analyzingTrainingData')}
            </p>
          </div>
        ) : proposal.data === undefined || lead === undefined ? (
          <EmptyState action={{ label: t('onboarding.retry'), onClick: () => void proposal.refetch() }}>
            {t('onboarding.agentSuggestionsFailed')}
          </EmptyState>
        ) : (
          <TeamPicker proposal={proposal.data} withoutProvider={withoutProvider} onSubmit={handleSubmit} />
        )}
      </div>
    </Sheet>
  );
}
