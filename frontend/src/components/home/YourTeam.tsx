// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Today's Your team — one folded row of avatars and names whose Manage unfolds the lead, the other agents and the way to Discover (carnet#1528)
// ABOUTME: A row reopens the athlete's latest chat with that agent, else a new one; an athlete with no team is offered the onboarding picker, never a silent backfill (carnet#1098)

import { useId, useMemo, useState } from 'react';
import { clsx } from 'clsx';
import { Plus } from 'lucide-react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from '@pierre/i18n';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { Agent } from '@pierre/shared-types';
import { avatarSlot, formatList, initialsFor } from '@pierre/chat-utils';
import { coachesApi, userApi } from '../../services/api';
import { usePersonalConversation } from '../../hooks/useConversationList';
import { avatarShapeClass, avatarSlotClass } from '../chat/avatarSlots';
import { Section } from '../ui/Section';
import { EmptyState } from '../ui/EmptyState';
import { Sheet } from '../ui/Sheet';
import { TeamPicker, type TeamChoice } from '../onboarding/TeamPicker';
import { useTeamProposal } from '../onboarding/useTeamProposal';
import { useOnboardingStatus } from '../../hooks/useOnboardingStatus';
import { useMyCoach } from '../../hooks/useMyCoach';
import { conversationCapOf } from '../chat/conversationCap';
import { openChatWithAgent } from './openNewChat';
import type { TodayActions } from './personalToday';

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

function AgentAvatar({ agent, className = 'h-8 w-8' }: { agent: Agent; className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={clsx(
        'flex shrink-0 items-center justify-center text-xs font-semibold',
        className,
        avatarShapeClass('circle'),
        avatarSlotClass(avatarSlot({ id: agent.id, agent_id: agent.id, group_id: null, subject_user_id: null })),
      )}
    >
      {initialsFor(agent.title)}
    </span>
  );
}

/** The most avatars the folded row stacks; the names line still names everyone. */
const ROW_AVATARS = 3;

const ROW_CLASS =
  'flex w-full items-center gap-3 rounded-lg px-2 py-1.5 text-left transition-colors hover:bg-surface-container-low focus-ring disabled:opacity-60';

export function YourTeam({ actions, onNavigate, onPickTeam }: YourTeamProps) {
  const { t, i18n } = useTranslation();
  const queryClient = useQueryClient();
  const personal = usePersonalConversation();
  // The athlete's human coach, when a coach invited them: Your team ends on
  // them (carnet#1649). A person, so a line of its own beside the agents.
  const coach = useMyCoach();
  // What a refused row says: the active-conversation cap, or that the chat
  // could not be opened.
  const [openError, setOpenError] = useState<string | null>(null);
  // The folded row unfolds into the team's rows on « Manage ».
  const [managing, setManaging] = useState(false);
  const headingId = useId();
  const listId = useId();
  const agentsQuery = useQuery({
    queryKey: QUERY_KEYS.coaches.list(),
    queryFn: () => coachesApi.list(),
    staleTime: AGENT_LIST_STALE_MS,
  });
  const { lead, members } = useMemo(
    () => orderTeam(agentsQuery.data?.agents ?? [], personal.agentId),
    [agentsQuery.data, personal.agentId],
  );

  // A row opens the athlete's chat with that agent, as a messenger opens a
  // contact: their latest conversation with it, else a new one beside the
  // personal thread (carnet#1473) — and lands on it.
  const openChat = useMutation({
    mutationFn: (agentId: string) => openChatWithAgent(queryClient, agentId),
    onSuccess: ({ id, dividerId }) => {
      setOpenError(null);
      actions.openTopic(id, dividerId);
    },
    onError: (error) => {
      const cap = conversationCapOf(error);
      setOpenError(
        cap ? t('chat.conversationLimitBody', { limit: cap.limit ?? '' }) : t('app.couldNotOpenChat'),
      );
    },
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

  if (team.length === 0) {
    return (
      <Section title={t('home.team.title')} headingLevel={3} actions={findMore} data-testid="home-team">
        <EmptyState
          data-testid="home-team-empty"
          action={{ label: t('home.team.pick'), onClick: onPickTeam, 'data-testid': 'home-team-pick' }}
        >
          {t('home.team.empty')}
        </EmptyState>
      </Section>
    );
  }

  // One row, folded (carnet#1528, P5·C): the team's avatars, everyone's
  // names and « Manage », which unfolds the rows that open a chat with each
  // agent, Add an agent and Find more.
  return (
    <section aria-labelledby={headingId} data-testid="home-team">
      <div className="flex items-center gap-3">
        <span aria-hidden="true" className="flex shrink-0 items-center">
          {team.slice(0, ROW_AVATARS).map((agent, index) => (
            <AgentAvatar
              key={agent.id}
              agent={agent}
              className={clsx('h-7 w-7 ring-2 ring-surface', index > 0 && '-ml-2')}
            />
          ))}
        </span>
        <div className="min-w-0 flex-1">
          <h3 id={headingId} className="text-sm font-semibold text-on-surface">
            {t('home.team.title')}
          </h3>
          <p data-testid="home-team-names" className="truncate text-xs text-on-surface-variant">
            {formatList(
              team.map((agent) => agent.title),
              i18n.language,
            )}
          </p>
        </div>
        <button
          type="button"
          data-testid="home-team-manage"
          aria-label={t('home.team.manageAria')}
          aria-expanded={managing}
          aria-controls={listId}
          onClick={() => setManaging((open) => !open)}
          className="shrink-0 rounded px-0.5 text-sm font-medium text-primary transition-colors hover:text-primary-hover focus-ring touch-target"
        >
          {t('home.team.manage')}
        </button>
      </div>
      {managing && (
        <div id={listId} className="mt-3">
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
                  disabled={openChat.isPending}
                  onClick={() => openChat.mutate(agent.id)}
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
        <div className="mt-1">{findMore}</div>
        </div>
      )}
      {coach !== null && (
        <p className="mt-2 text-xs text-on-surface-variant" data-testid="home-team-coach">
          {t('humanCoach.inviteJourney.yourCoach', { name: coach.name })}
        </p>
      )}
      {openError && (
        <p role="alert" className="mt-2 text-sm text-error">
          {openError}
        </p>
      )}
    </section>
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
