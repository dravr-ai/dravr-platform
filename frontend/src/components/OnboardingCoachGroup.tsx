// ABOUTME: Onboarding step — a coach names their group, picks the agent its athletes talk to, and leaves with the invite
// ABOUTME: Coach access is never granted here (ADR-018): without it the group is made coachless and the step says so

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useEffect, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import QRCode from 'qrcode';
import type { Agent, CoachingGroup } from '@pierre/shared-types';
import { coachCategoryLabelKey } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { chatApi, coachesApi, groupsApi } from '../services/api';
import { QUERY_KEYS } from '../constants/queryKeys';
import { Button, Input } from './ui';
import OnboardingShell from './OnboardingShell';

/**
 * How long the onboarding athlete invite stays valid. Longer than the 7 days a
 * chat-issued invite gets: a coach sets up during onboarding, often before
 * the athlete is ready to sign up, and a week is easily missed.
 */
const ONBOARDING_INVITE_DAYS = 30;

/** Where the group setup stands: to name, its agent to pick, or made and ready to share. */
type Phase =
  | { kind: 'name' }
  | { kind: 'agent' }
  | { kind: 'share'; group: CoachingGroup; link: string };

/**
 * The coach's group step: name the group, pick its agent, then share the
 * athlete invite.
 *
 * The agent is the one the athletes talk to in the group's thread, chosen from
 * the catalogue here rather than inherited from the coach's own selection: a
 * coach who does not train never picked one, and one ranked on the coach's own
 * rides says nothing about the athletes they coach.
 *
 * The group is created with the coach as its human coach when they hold coach
 * access (`manages_roster`); the server decides, never this screen. A group
 * that comes back without a coach is shown as access pending, with the way to
 * ask for it. The coach's chat thread for the group is opened here too, since
 * a group nobody has a thread for is a group the coach cannot find.
 */
export default function OnboardingCoachGroup({
  userDisplayName,
  onComplete,
}: {
  userDisplayName?: string | null;
  onComplete: (status: 'complete' | 'skipped') => void;
}) {
  const { t } = useTranslation();
  const [name, setName] = useState('');
  const [phase, setPhase] = useState<Phase>({ kind: 'name' });
  const [agentId, setAgentId] = useState<string | null>(null);
  const [created, setCreated] = useState<CoachingGroup | null>(null);
  const [working, setWorking] = useState(false);
  const [failed, setFailed] = useState(false);

  const create = async () => {
    const trimmed = name.trim();
    if (!trimmed || !agentId || working) return;
    setWorking(true);
    setFailed(false);
    try {
      // A retry after a later call failed reuses the group already made rather
      // than making a second one.
      const group =
        created ??
        (await groupsApi.createGroup({ name: trimmed, agent_id: agentId, coach_is_me: true }));
      setCreated(group);
      await chatApi.createConversation({ group_id: group.id, agent_id: group.agent_id });
      const invite = await groupsApi.createInvite(group.id, {
        expires_in_days: ONBOARDING_INVITE_DAYS,
      });
      const link = `${window.location.origin}/groups/join/${encodeURIComponent(invite.code)}`;
      setPhase({ kind: 'share', group, link });
    } catch {
      setFailed(true);
    } finally {
      setWorking(false);
    }
  };

  if (phase.kind === 'share') {
    return (
      <OnboardingShell heading={t('onboarding.groupInviteHeading')}>
        <InviteShare group={phase.group} link={phase.link} onDone={() => onComplete('complete')} />
      </OnboardingShell>
    );
  }

  if (phase.kind === 'agent') {
    return (
      <OnboardingShell heading={t('onboarding.groupAgentHeading')}>
        <p className="mt-3 text-sm text-on-surface-variant text-center">
          {t('onboarding.groupAgentIntro')}
        </p>
        <AgentPicker selected={agentId} onSelect={setAgentId} disabled={working} />
        {failed ? (
          <p className="mt-4 text-sm text-error text-center" role="alert">
            {t('onboarding.groupCreateFailed')}
          </p>
        ) : null}
        <div className="mt-8 space-y-3">
          <Button
            variant="primary"
            onClick={() => void create()}
            disabled={working || agentId === null}
            className="w-full"
          >
            {working ? t('onboarding.groupCreating') : t('onboarding.groupCreate')}
          </Button>
          {/* Once the group exists its name and agent are fixed: going back
              would only offer edits the retry cannot apply. */}
          {created ? null : (
            <button
              type="button"
              onClick={() => setPhase({ kind: 'name' })}
              disabled={working}
              className="w-full text-sm font-medium text-on-surface-variant hover:text-on-surface underline-offset-2 hover:underline transition-colors"
            >
              {t('common.back')}
            </button>
          )}
        </div>
      </OnboardingShell>
    );
  }

  return (
    <OnboardingShell
      heading={
        userDisplayName
          ? t('onboarding.welcomeNamed', { name: userDisplayName })
          : t('onboarding.groupHeading')
      }
    >
      <p className="mt-3 text-sm text-on-surface-variant text-center">{t('humanCoach.groupStepIntro')}</p>
      <form
        className="mt-8 space-y-6"
        onSubmit={(e) => {
          e.preventDefault();
          if (name.trim() !== '') setPhase({ kind: 'agent' });
        }}
      >
        <Input
          label={t('onboarding.groupNameLabel')}
          placeholder={t('onboarding.groupNamePlaceholder')}
          value={name}
          maxLength={100}
          onChange={(e) => setName(e.target.value)}
          data-testid="onboarding-group-name"
        />
        <div className="space-y-3">
          <Button type="submit" variant="primary" disabled={name.trim() === ''} className="w-full">
            {t('common.next')}
          </Button>
          <button
            type="button"
            onClick={() => onComplete('skipped')}
            className="w-full text-sm font-medium text-on-surface-variant hover:text-on-surface underline-offset-2 hover:underline transition-colors"
          >
            {t('onboarding.groupLater')}
          </button>
        </div>
      </form>
    </OnboardingShell>
  );
}

/**
 * The catalogue as a single-choice list. Unranked on purpose: the coach's own
 * activities are no evidence of what their athletes need, so nothing here is
 * offered as a recommendation.
 */
function AgentPicker({
  selected,
  onSelect,
  disabled,
}: {
  selected: string | null;
  onSelect: (agentId: string) => void;
  disabled: boolean;
}) {
  const { t } = useTranslation();
  const { data, isLoading, isError } = useQuery({
    queryKey: QUERY_KEYS.coaches.list(),
    queryFn: () => coachesApi.list(),
  });
  const agents: Agent[] = (data?.agents ?? []).filter((a) => !a.is_hidden);

  if (isLoading) {
    return (
      <p className="mt-8 text-sm text-on-surface-variant text-center">
        {t('discover.loadingAgents')}
      </p>
    );
  }
  if (isError) {
    return (
      <p className="mt-8 text-sm text-error text-center" role="alert">
        {t('app.failedLoadAgents')}
      </p>
    );
  }
  if (agents.length === 0) {
    return (
      <p className="mt-8 text-sm text-on-surface-variant text-center">
        {t('app.noAgentsAvailable')}
      </p>
    );
  }

  return (
    <div
      role="radiogroup"
      aria-label={t('onboarding.groupAgentHeading')}
      className="mt-8 space-y-3"
      data-testid="onboarding-group-agents"
    >
      {agents.map((agent) => {
        const checked = agent.id === selected;
        return (
          <button
            key={agent.id}
            type="button"
            role="radio"
            aria-checked={checked}
            disabled={disabled}
            onClick={() => onSelect(agent.id)}
            data-testid={`onboarding-group-agent-${agent.id}`}
            className={`w-full rounded-xl border px-5 py-4 text-left transition-colors ${
              checked
                ? 'border-primary bg-surface-container'
                : 'border-outline-variant bg-surface-container-low hover:bg-surface-container'
            }`}
          >
            <span className="block font-display font-semibold text-base text-on-surface truncate">
              {agent.title}
            </span>
            <span className="mt-0.5 block text-xs text-on-surface-variant">
              {t(coachCategoryLabelKey(agent.category))}
            </span>
            {agent.description ? (
              <span className="mt-2 block text-sm text-on-surface-variant line-clamp-2">
                {agent.description}
              </span>
            ) : null}
          </button>
        );
      })}
    </div>
  );
}

/** The invite link, its QR code, and the honest coach-access state. */
function InviteShare({
  group,
  link,
  onDone,
}: {
  group: CoachingGroup;
  link: string;
  onDone: () => void;
}) {
  const { t } = useTranslation();
  const [qr, setQr] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const canShare = typeof navigator !== 'undefined' && typeof navigator.share === 'function';

  useEffect(() => {
    let live = true;
    // The QR ground stays white in both themes: a phone camera needs it.
    QRCode.toDataURL(link, { margin: 1, width: 220 })
      .then((url) => {
        if (live) setQr(url);
      })
      // No QR is no loss: the link beside it is the same invite.
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [link]);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(link);
      setCopied(true);
    } catch {
      // The link stays on screen to copy by hand.
    }
  };

  return (
    <>
      <p className="mt-3 text-sm text-on-surface-variant text-center">
        {t('onboarding.groupInviteIntro')}
      </p>

      {qr ? (
        <div className="mt-8 flex justify-center">
          <img
            src={qr}
            alt={t('onboarding.groupInviteQrAlt')}
            width={220}
            height={220}
            className="rounded-md bg-white"
            data-testid="onboarding-group-qr"
          />
        </div>
      ) : null}

      <p
        className="mt-6 break-all text-center font-mono text-sm text-on-surface"
        data-testid="onboarding-group-link"
      >
        {link}
      </p>

      <div className="mt-6 flex gap-3">
        <Button variant="secondary" onClick={() => void copy()} className="flex-1">
          {copied ? t('onboarding.groupLinkCopied') : t('onboarding.groupCopyLink')}
        </Button>
        {canShare ? (
          <Button
            variant="secondary"
            onClick={() => void navigator.share({ title: group.name, url: link }).catch(() => {})}
            className="flex-1"
          >
            {t('onboarding.groupShare')}
          </Button>
        ) : null}
      </div>

      {group.coach_user_id ? null : (
        <div
          className="mt-8 border-t ghost-border-faint pt-6"
          data-testid="onboarding-group-access-pending"
        >
          <h2 className="font-display font-semibold text-lg text-on-surface">
            {t('humanCoach.accessPendingHeading')}
          </h2>
          <p className="mt-2 text-sm text-on-surface-variant">
            {t('humanCoach.accessPendingBody')}
          </p>
          <a
            href="mailto:support@dravr.ai"
            className="mt-3 inline-block text-sm font-medium text-primary underline-offset-2 hover:underline"
          >
            {t('onboarding.groupAccessContact')}
          </a>
        </div>
      )}

      <Button variant="primary" onClick={onDone} className="mt-8 w-full">
        {t('onboarding.groupDone')}
      </Button>
    </>
  );
}
