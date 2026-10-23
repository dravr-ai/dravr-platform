// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The team picker — what we read, the lead with Change, ticked members, Browse all agents, one Start
// ABOUTME: Shared by onboarding's Your team step and Home's Pick your team sheet; it writes nothing itself, onSubmit does

import { useState } from 'react';
import { Trans, useTranslation } from '@pierre/i18n';
import { initialsFor } from '@pierre/chat-utils';
import { activitySportLabelKey, coachCategoryLabelKey } from '@pierre/shared-constants';
import type { AgentProposalResponse, StoreAgent, TeamMember } from '@pierre/shared-types';
import { Button, Checkbox, Sheet } from '../ui';
import StoreScreen from '../StoreScreen';
import type { TeamStepChoice } from '../../hooks/useOnboardingState';

/**
 * What Start or Skip hands the host to write: the step's choice with the lead
 * and every agent joining (the lead included) always set.
 */
export type TeamChoice = Required<TeamStepChoice>;

/** One agent as the picker shows it, from the proposal or picked in the Discover sheet. */
interface TeamEntry {
  id: string;
  title: string;
  category: string;
  /** The source line's second half ("from your goal"), already in the athlete's language. */
  source?: string;
  reason?: string;
}

interface MemberEntry extends TeamEntry {
  ticked: boolean;
}

// An agent's avatar sits on its pillar's tint with the pillar's bound ink
// (DESIGN.md §2): hue as ground, never as text on paper.
const AVATAR_BY_CATEGORY: Record<string, string> = {
  training: 'bg-activity/15 text-on-activity-container',
  nutrition: 'bg-nutrition/15 text-on-nutrition-container',
  recipes: 'bg-nutrition/15 text-on-nutrition-container',
  recovery: 'bg-recovery/15 text-on-recovery-container',
  mobility: 'bg-mobility/15 text-on-mobility-container',
};
const AVATAR_FALLBACK = 'bg-primary-container text-on-primary-container';

function fromStore(agent: StoreAgent, source?: string): TeamEntry {
  return { id: agent.id, title: agent.title, category: agent.category, source };
}

/** Which sheet is open over the picker, if any. */
type OpenSheet = 'lead' | 'browse' | null;

export interface TeamPickerProps {
  proposal: AgentProposalResponse;
  /**
   * The athlete has no provider connected: the empty "what we read" says so
   * instead of waiting on a provider to sync.
   */
  withoutProvider?: boolean;
  /**
   * Write the team. Resolves once it is saved; a rejection keeps the picker
   * on screen with an inline error and a retry.
   */
  onSubmit: (choice: TeamChoice) => Promise<void>;
}

/**
 * The team picker (carnet#1095, design canvas "Your team").
 *
 * One lead, always on the team and so without a tick box, which "Change"
 * swaps through the Discover sheet; the members the server's rules picked,
 * pre-ticked or not as it said; "Browse all agents" to add any other agent,
 * which joins the list ticked. One filled button starts with the lead and every
 * ticked member; Skip starts with the lead alone. Nothing is installed until
 * the host's `onSubmit` writes the whole team in one call.
 */
export function TeamPicker({ proposal, withoutProvider = false, onSubmit }: TeamPickerProps) {
  const { t } = useTranslation();
  const [lead, setLead] = useState<TeamEntry | null>(() => initialLead(proposal));
  const [members, setMembers] = useState<MemberEntry[]>(() =>
    initialMembers(proposal, (member) => memberSource(member, t)),
  );
  const [sheet, setSheet] = useState<OpenSheet>(null);
  const [saving, setSaving] = useState(false);
  const [failed, setFailed] = useState<TeamChoice['status'] | null>(null);

  if (!lead) return null;

  const ticked = members.filter((member) => member.ticked);
  const count = 1 + ticked.length;

  const submit = async (status: TeamChoice['status']) => {
    setSaving(true);
    setFailed(null);
    const teamAgentIds = status === 'skipped' ? [lead.id] : [lead.id, ...ticked.map((m) => m.id)];
    try {
      await onSubmit({ status, leadAgentId: lead.id, teamAgentIds });
    } catch {
      setFailed(status);
    } finally {
      setSaving(false);
    }
  };

  const toggle = (id: string) =>
    setMembers((current) => current.map((m) => (m.id === id ? { ...m, ticked: !m.ticked } : m)));

  // The picked agent leads; a member it was leaves the list, since the lead
  // is on the team without a tick.
  const chooseLead = (agent: StoreAgent) => {
    setLead(fromStore(agent));
    setMembers((current) => current.filter((m) => m.id !== agent.id));
    setSheet(null);
  };

  // An agent added from Discover joins the list ticked; one already listed is
  // ticked again rather than listed twice. The sheet stays open for the next.
  const addMember = (agent: StoreAgent) => {
    if (agent.id === lead.id) return;
    setMembers((current) =>
      current.some((m) => m.id === agent.id)
        ? current.map((m) => (m.id === agent.id ? { ...m, ticked: true } : m))
        : [...current, { ...fromStore(agent, t('onboarding.team.source.discover')), ticked: true }],
    );
  };

  return (
    <div className="flex flex-col gap-7">
      <WhatWeRead proposal={proposal} withoutProvider={withoutProvider} />

      <section aria-labelledby="team-lead-heading">
        <h2 id="team-lead-heading" className="text-[13px] font-semibold text-on-surface">
          {t('onboarding.team.leadHeading')}
        </h2>
        <div
          data-testid="team-lead"
          className="mt-2 flex items-start gap-3 rounded-xl border ghost-border bg-surface-container-lowest px-5 py-4"
        >
          <AgentAvatar entry={lead} />
          <div className="min-w-0 flex-1">
            <div className="flex items-baseline justify-between gap-3">
              <h3 className="truncate font-display text-base font-semibold text-on-surface">{lead.title}</h3>
              <button
                type="button"
                onClick={() => setSheet('lead')}
                aria-label={t('onboarding.team.changeLeadAria')}
                disabled={saving}
                className="btn-base btn-tertiary btn-sm touch-target shrink-0 px-0"
              >
                {t('onboarding.team.changeLead')}
              </button>
            </div>
            <p className="text-xs text-outline">{t(coachCategoryLabelKey(lead.category))}</p>
            {lead.reason ? <p className="mt-1 text-[13px] text-on-surface-variant">{lead.reason}</p> : null}
          </div>
        </div>
      </section>

      <fieldset disabled={saving}>
        <legend className="text-[13px] font-semibold text-on-surface">{t('onboarding.team.membersLegend')}</legend>
        {members.length === 0 ? (
          <p className="mt-2 text-[13px] text-on-surface-variant">{t('onboarding.team.noMembers')}</p>
        ) : (
          <div className="mt-2 rounded-xl border ghost-border bg-surface-container-lowest">
            {members.map((member) => (
              <div
                key={member.id}
                data-testid="team-member"
                className="border-t ghost-border-faint px-5 py-4 first:border-t-0"
              >
                <Checkbox
                  checked={member.ticked}
                  onChange={() => toggle(member.id)}
                  className="mt-3"
                  label={<MemberLabel entry={member} />}
                />
              </div>
            ))}
          </div>
        )}
      </fieldset>

      <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-1">
        <p className="text-xs text-outline">{t('onboarding.team.footnote')}</p>
        <button
          type="button"
          onClick={() => setSheet('browse')}
          disabled={saving}
          className="btn-base btn-tertiary btn-sm touch-target px-0"
        >
          {t('onboarding.team.browseAll')}
        </button>
      </div>

      {failed ? (
        <div role="alert" className="flex flex-wrap items-center gap-x-3 gap-y-1">
          <p className="text-[13px] text-error">{t('onboarding.team.saveFailed')}</p>
          <button
            type="button"
            onClick={() => void submit(failed)}
            disabled={saving}
            className="btn-base btn-tertiary btn-sm touch-target px-0"
          >
            {t('onboarding.retry')}
          </button>
        </div>
      ) : null}

      <div className="flex flex-col items-stretch gap-1">
        <Button variant="primary" onClick={() => void submit('complete')} disabled={saving}>
          {saving ? (
            t('onboarding.starting')
          ) : (
            <Trans i18nKey="onboarding.team.start" values={{ count }} components={{ num: <span className="font-mono" /> }} />
          )}
        </Button>
        <button
          type="button"
          onClick={() => void submit('skipped')}
          disabled={saving}
          className="btn-base btn-tertiary touch-target self-center"
        >
          {t('onboarding.team.skip', { lead: lead.title })}
        </button>
      </div>

      {sheet === 'lead' ? (
        <Sheet side="right" width="full" title={t('onboarding.team.leadSheetTitle')} onClose={() => setSheet(null)}>
          <StoreScreen mode="pickLead" pickedIds={[lead.id]} onPick={chooseLead} />
        </Sheet>
      ) : null}
      {sheet === 'browse' ? (
        <Sheet side="right" width="full" title={t('onboarding.team.browseSheetTitle')} onClose={() => setSheet(null)}>
          <StoreScreen
            mode="pickTeam"
            pickedIds={[lead.id, ...members.filter((m) => m.ticked).map((m) => m.id)]}
            onPick={addMember}
          />
        </Sheet>
      ) : null}
    </div>
  );
}

/** "What we read": the window, the count in mono, the main sport, the goal and the sport mix. */
function WhatWeRead({ proposal, withoutProvider }: { proposal: AgentProposalResponse; withoutProvider: boolean }) {
  const { t } = useTranslation();
  const { profile, goal } = proposal;
  // A sport as the profile spells it, in the athlete's language when the
  // vocabulary knows it and as spelled on the wire when it does not.
  const sportLabel = (sport: string): string => {
    const key = activitySportLabelKey(sport);
    return key ? t(key) : sport;
  };
  const num = { num: <span className="font-mono" /> };
  const count = profile.total_activities;

  return (
    <section
      aria-label={t('onboarding.team.whatWeReadAria')}
      className="rounded-xl border ghost-border bg-surface-container-lowest px-5 py-4"
    >
      {profile.has_profile && count > 0 ? (
        <>
          <p className="text-sm text-on-surface">
            {count === 1 ? (
              <Trans i18nKey="onboarding.team.summaryOne" values={{ days: profile.window_days }} components={num} />
            ) : profile.primary_sport ? (
              <Trans
                i18nKey="onboarding.team.summary"
                values={{ days: profile.window_days, count, sport: sportLabel(profile.primary_sport) }}
                components={num}
              />
            ) : (
              <Trans
                i18nKey="onboarding.team.summaryNoSport"
                values={{ days: profile.window_days, count }}
                components={num}
              />
            )}
            {goal ? <> {t('onboarding.team.goalLine', { goal })}</> : null}
          </p>
          {profile.sport_mix.length > 0 ? (
            <div className="mt-3 space-y-1.5">
              {profile.sport_mix.map((s) => {
                const percent = Math.round(s.share * 100);
                return (
                  <div key={s.sport} className="grid grid-cols-[72px_1fr_40px] items-center gap-3">
                    <span className="truncate text-xs text-on-surface-variant" title={sportLabel(s.sport)}>
                      {sportLabel(s.sport)}
                    </span>
                    <div className="h-1.5 overflow-hidden rounded-full bg-surface-container-low">
                      <div className="h-full rounded-full bg-primary" style={{ width: `${percent}%` }} />
                    </div>
                    <span className="text-right font-mono text-xs text-on-surface-variant">{percent}%</span>
                  </div>
                );
              })}
            </div>
          ) : null}
        </>
      ) : (
        <p className="text-sm text-on-surface-variant">
          {t(withoutProvider ? 'onboarding.agentProposalNoProvider' : 'onboarding.agentProposalNoActivities')}
          {goal ? <> {t('onboarding.team.goalLine', { goal })}</> : null}
        </p>
      )}
    </section>
  );
}

function AgentAvatar({ entry }: { entry: TeamEntry }) {
  return (
    <span
      aria-hidden="true"
      className={`flex h-10 w-10 shrink-0 select-none items-center justify-center rounded-full font-display text-xs font-semibold ${
        AVATAR_BY_CATEGORY[entry.category] ?? AVATAR_FALLBACK
      }`}
    >
      {initialsFor(entry.title)}
    </span>
  );
}

/** A member row's label: avatar, title, `<category> · <source>` and the one-line reason. */
function MemberLabel({ entry }: { entry: MemberEntry }) {
  const { t } = useTranslation();
  const category = t(coachCategoryLabelKey(entry.category));
  return (
    <span className="flex items-start gap-3">
      <AgentAvatar entry={entry} />
      <span className="min-w-0">
        <span className="block truncate font-display text-base font-semibold text-on-surface">{entry.title}</span>
        <span className="block text-xs text-outline">
          {entry.source ? `${category} · ${entry.source}` : category}
        </span>
        {entry.reason ? (
          <span className="mt-1 block text-[13px] text-on-surface-variant">{entry.reason}</span>
        ) : null}
      </span>
    </span>
  );
}

/** The lead the server proposed: the team's, or the re-rank's first from a server that predates teams. */
function initialLead(proposal: AgentProposalResponse): TeamEntry | null {
  const proposed = proposal.team?.lead ?? proposal.agents[0];
  if (!proposed) return null;
  const { agent, reason } = proposed;
  return { id: agent.id, title: agent.title, category: agent.category, reason: reason || undefined };
}

function initialMembers(
  proposal: AgentProposalResponse,
  sourceOf: (member: TeamMember) => string | undefined,
): MemberEntry[] {
  return (proposal.team?.members ?? []).map((member) => ({
    id: member.agent.id,
    title: member.agent.title,
    category: member.agent.category,
    source: sourceOf(member),
    reason: member.reason || undefined,
    ticked: member.preselected,
  }));
}

/** Why a member is on the list, in the athlete's language: the rule that picked it. */
function memberSource(
  member: TeamMember,
  t: (key: string, options?: Record<string, unknown>) => string,
): string | undefined {
  switch (member.rule) {
    case 'goal':
      return t('onboarding.team.source.goal');
    case 'long_sessions':
      return t('onboarding.team.source.longSessions');
    case 'sleep_source':
      return member.source ? t('onboarding.team.source.sleepSource', { provider: member.source }) : undefined;
    case 'strength':
      return t('onboarding.team.source.strength');
    default:
      return undefined;
  }
}
