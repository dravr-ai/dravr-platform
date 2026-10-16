// ABOUTME: Onboarding step — a coach names their group, picks the agent its athletes talk to, and leaves with the invite
// ABOUTME: Coach access is never granted here (ADR-018): a coachless group offers a one-tap request a super-admin decides

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useEffect, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import QRCode from 'qrcode';
import type { Agent, CoachingGroup } from '@pierre/shared-types';
import { coachCategoryLabelKey, isCoachFacing } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { coachesApi } from '../services/api';
import { useCoachGroupSetup } from '../hooks/useGroups';
import { QUERY_KEYS } from '../constants/queryKeys';
import { Button, Input } from './ui';
import OnboardingShell from './OnboardingShell';
import CoachAccessRequest from './CoachAccessRequest';

/** Where the group setup stands before it is made: to name, or its agent to pick. */
type Phase = { kind: 'name' } | { kind: 'agent' };

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
 * that comes back without a coach is shown as access pending, with a one-tap
 * request a super-admin grants or declines (carnet#738); a grant makes the
 * coach this group's coach with no further step. The coach's chat thread for
 * the group is opened here too, since a group nobody has a thread for is a
 * group the coach cannot find, and "Go to my group" lands in that thread.
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
  const { create, working, failed, group: created, conversationId, inviteCode } = useCoachGroupSetup();

  if (created && conversationId && inviteCode) {
    const link = `${window.location.origin}/groups/join/${encodeURIComponent(inviteCode)}`;
    return (
      <OnboardingShell heading={t('onboarding.groupInviteHeading')}>
        <InviteShare
          group={created}
          link={link}
          onDone={() => {
            // The step is recorded before the hand-off. The dashboard reads
            // `#chat/<id>` when it mounts, which opens the group's thread in
            // the Groups tab instead of Home.
            onComplete('complete');
            window.location.hash = `#chat/${encodeURIComponent(conversationId)}`;
          }}
        />
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
            onClick={() => {
              if (agentId !== null) void create(name, agentId);
            }}
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
 * offered as a recommendation. Coach-facing agents are left out: the athletes
 * talk to this one.
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
  const agents: Agent[] = (data?.agents ?? []).filter((a) => !a.is_hidden && !isCoachFacing(a));

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
          <CoachAccessRequest groupId={group.id} />
        </div>
      )}

      <Button
        variant="primary"
        onClick={onDone}
        className="mt-8 w-full"
        data-testid="onboarding-group-done"
      >
        {t('onboarding.groupDone')}
      </Button>
    </>
  );
}
