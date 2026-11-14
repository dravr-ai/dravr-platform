// ABOUTME: Onboarding steps "Your group" and "Invite" — a coach names their athlete's group with its agent on one screen, then shares the link
// ABOUTME: Coach access is never granted here (ADR-018): the invite's seat card says what the server holds, with a quiet request

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useEffect, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import QRCode from 'qrcode';
import { Check } from 'lucide-react';
import type { Agent, CoachingGroup } from '@pierre/shared-types';
import {
  ONBOARDING_INVITE_DAYS,
  canRequestCoachAccess,
  coachCategoryLabelKey,
  coachSeatMoment,
  coachSeatState,
  isCoachFacing,
} from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { coachesApi, userApi } from '../services/api';
import { useCoachGroupSetup } from '../hooks/useGroups';
import { useAuth } from '../hooks/useAuth';
import { QUERY_KEYS } from '../constants/queryKeys';
import { Button, Input } from './ui';
import OnboardingShell from './OnboardingShell';
import CoachAccessRequest from './CoachAccessRequest';
import { CheckDisc } from './OnboardingCoachSeat';

/**
 * The coach's group, then its invite: two segments of the progress bar, one
 * component, so the group just made and its link survive the step changing
 * under it (`onCreated` moves the bar on; `onComplete` records the step).
 *
 * "Your group" is one screen: the athlete, their goal if the coach gives one,
 * the group's name filled in from them, and the agent the athletes talk to in
 * the thread — defaulted, with « Changer » opening the catalogue. The agent is
 * chosen for the group, never inherited from the coach's own selection: a
 * coach who does not train never picked one.
 *
 * The group is created with the coach as its human coach when they hold coach
 * access (`manages_roster`); the server decides, never this screen. The
 * coach's thread for the group is opened here too, since a group nobody has a
 * thread for is a group the coach cannot find.
 *
 * Where the invite leaves for follows the seat: a coach whose access is active
 * (the server made them the group's coach — invited by the team, or a
 * TrainingPeaks coach account) opens their coach Home, where the athlete they
 * just invited waits as a row; a coach still without it goes to the group's
 * thread. A coach the team invited (`seatSettledByInvite`) is also told on
 * the group screen that the seat is already active.
 */
export default function OnboardingCoachGroup({
  onCreated,
  onComplete,
  seatSettledByInvite = false,
}: {
  onCreated?: () => void;
  onComplete: (status: 'complete' | 'skipped') => void;
  seatSettledByInvite?: boolean;
}) {
  const { t } = useTranslation();
  const [athlete, setAthlete] = useState('');
  const [goal, setGoal] = useState('');
  // The group's name follows the athlete until the coach types their own.
  const [typedName, setTypedName] = useState<string | null>(null);
  const [pickedAgentId, setPickedAgentId] = useState<string | null>(null);
  const [changingAgent, setChangingAgent] = useState(false);
  const { create, working, failed, group: created, conversationId, inviteCode } = useCoachGroupSetup();
  const agents = useGroupAgents();

  const athleteName = athlete.trim();
  const goalText = goal.trim();
  const suggestedName =
    athleteName === ''
      ? ''
      : goalText === ''
        ? athleteName
        : t('onboarding.groupNameFromAthleteGoal', { athlete: athleteName, goal: goalText });
  const name = typedName ?? suggestedName;
  const agentId = pickedAgentId ?? defaultAgent(agents.list)?.id ?? null;
  const agent = agents.list.find((a) => a.id === agentId) ?? null;

  const ready = created !== null && conversationId !== null && inviteCode !== null;
  // Any active seat lands on the coach Home: the server made the coach the
  // group's coach only because they hold coach access.
  const toCoachHome = created?.coach_user_id != null;
  useEffect(() => {
    if (ready) onCreated?.();
  }, [ready, onCreated]);

  if (created && conversationId && inviteCode) {
    const link = `${window.location.origin}/groups/join/${encodeURIComponent(inviteCode)}`;
    return (
      <OnboardingShell
        heading={
          athleteName
            ? t('onboarding.groupInviteHeadingNamed', { name: athleteName })
            : t('onboarding.groupInviteHeading')
        }
      >
        <InviteShare
          group={created}
          link={link}
          athleteName={athleteName}
          toCoachHome={toCoachHome}
          onDone={() => {
            // The step is recorded before the hand-off. The dashboard reads
            // the hash when it mounts: `#home` opens the coach Home of a
            // coach whose seat is active, `#chat/<id>` the group's thread in
            // the Groups tab for one still without it.
            onComplete('complete');
            window.location.hash = toCoachHome ? '#home' : `#chat/${encodeURIComponent(conversationId)}`;
          }}
        />
      </OnboardingShell>
    );
  }

  // Once the group exists its name and agent are fixed: a retry after a later
  // call failed can apply no edit, so the fields lock.
  const locked = working || created !== null;
  return (
    <OnboardingShell heading={t('onboarding.groupHeading')}>
      <div className="mx-auto max-w-md">
        {seatSettledByInvite ? (
          <p
            className="mt-3 flex items-center justify-center gap-2 rounded-lg bg-success/10 px-3 py-2 text-sm text-on-surface"
            data-testid="onboarding-group-seat-active"
          >
            <Check aria-hidden="true" className="h-4 w-4 shrink-0 text-success" strokeWidth={2} />
            <span>{t('humanCoach.seatInvite.seatActive')}</span>
          </p>
        ) : null}
        <p className="mt-3 text-sm text-on-surface-variant text-center">{t('humanCoach.groupStepIntro')}</p>
        <form
          className="mt-8 space-y-6"
          onSubmit={(e) => {
            e.preventDefault();
            if (name.trim() !== '' && agentId !== null) void create(name, agentId, goalText);
          }}
        >
          <Input
            label={t('onboarding.groupAthleteLabel')}
            placeholder={t('onboarding.groupAthletePlaceholder')}
            value={athlete}
            maxLength={60}
            disabled={locked}
            onChange={(e) => setAthlete(e.target.value)}
            data-testid="onboarding-group-athlete"
          />
          <Input
            label={t('onboarding.groupGoalLabel')}
            placeholder={t('onboarding.groupGoalPlaceholder')}
            value={goal}
            maxLength={80}
            disabled={locked}
            onChange={(e) => setGoal(e.target.value)}
            data-testid="onboarding-group-goal"
          />
          <Input
            label={t('onboarding.groupNameLabel')}
            placeholder={t('onboarding.groupNamePlaceholder')}
            helpText={typedName === null && suggestedName !== '' ? t('onboarding.groupNameHint') : undefined}
            value={name}
            maxLength={100}
            disabled={locked}
            onChange={(e) => setTypedName(e.target.value)}
            data-testid="onboarding-group-name"
          />

          <AgentField
            agents={agents}
            agent={agent}
            changing={changingAgent}
            disabled={locked}
            onChange={() => setChangingAgent(true)}
            onSelect={(id) => {
              setPickedAgentId(id);
              setChangingAgent(false);
            }}
          />

          {failed ? (
            <p className="text-sm text-error text-center" role="alert">
              {t('onboarding.groupCreateFailed')}
            </p>
          ) : null}

          <div className="flex flex-col items-center gap-3">
            <Button
              type="submit"
              variant="primary"
              disabled={working || name.trim() === '' || agentId === null}
              className="w-full"
            >
              {working ? t('onboarding.groupCreating') : t('onboarding.groupCreate')}
            </Button>
            {created ? null : (
              <Button type="button" variant="tertiary" onClick={() => onComplete('skipped')} disabled={working}>
                {t('onboarding.groupLater')}
              </Button>
            )}
          </div>
        </form>
      </div>
    </OnboardingShell>
  );
}

/** The athlete-facing catalogue the group's agent is chosen from. */
function useGroupAgents() {
  const { data, isLoading, isError } = useQuery({
    queryKey: QUERY_KEYS.coaches.list(),
    queryFn: () => coachesApi.list(),
  });
  // Coach-facing agents are left out: the athletes talk to this one.
  const list: Agent[] = (data?.agents ?? []).filter((a) => !a.is_hidden && !isCoachFacing(a));
  return { list, isLoading, isError };
}

/**
 * The agent a new group starts with: the endurance generalist, which reads
 * load, recovery and key sessions whatever the sport, else the catalogue's
 * first. The coach's own activities are no evidence of what their athletes
 * need, so nothing is ranked on them.
 */
function defaultAgent(agents: Agent[]): Agent | undefined {
  return agents.find((a) => a.tags?.includes('endurance')) ?? agents[0];
}

/** The group's agent as one row with « Changer », or the catalogue while changing it. */
function AgentField({
  agents,
  agent,
  changing,
  disabled,
  onChange,
  onSelect,
}: {
  agents: ReturnType<typeof useGroupAgents>;
  agent: Agent | null;
  changing: boolean;
  disabled: boolean;
  onChange: () => void;
  onSelect: (agentId: string) => void;
}) {
  const { t } = useTranslation();
  const label = (
    <p className="text-sm font-medium text-on-surface">{t('onboarding.groupAgentLabel')}</p>
  );

  if (agents.isLoading) {
    return (
      <div>
        {label}
        <p className="mt-2 text-sm text-on-surface-variant">{t('discover.loadingAgents')}</p>
      </div>
    );
  }
  if (agents.isError) {
    return (
      <div>
        {label}
        <p className="mt-2 text-sm text-error" role="alert">
          {t('app.failedLoadAgents')}
        </p>
      </div>
    );
  }
  if (agents.list.length === 0 || agent === null) {
    return (
      <div>
        {label}
        <p className="mt-2 text-sm text-on-surface-variant">{t('app.noAgentsAvailable')}</p>
      </div>
    );
  }

  if (changing) {
    return (
      <div>
        {label}
        <div
          role="radiogroup"
          aria-label={t('onboarding.groupAgentLabel')}
          className="mt-2 space-y-2"
          data-testid="onboarding-group-agents"
        >
          {agents.list.map((option) => {
            const checked = option.id === agent.id;
            return (
              <button
                key={option.id}
                type="button"
                role="radio"
                aria-checked={checked}
                disabled={disabled}
                onClick={() => onSelect(option.id)}
                data-testid={`onboarding-group-agent-${option.id}`}
                className={`w-full rounded-xl border px-4 py-3 text-left transition-colors ${
                  checked
                    ? 'border-primary bg-surface-container'
                    : 'border-outline-variant bg-surface-container-low hover:bg-surface-container'
                }`}
              >
                <span className="block text-sm font-semibold text-on-surface truncate">{option.title}</span>
                <span className="mt-0.5 block text-xs text-on-surface-variant">
                  {t(coachCategoryLabelKey(option.category))}
                </span>
                {option.description ? (
                  <span className="mt-1 block text-sm text-on-surface-variant line-clamp-2">
                    {option.description}
                  </span>
                ) : null}
              </button>
            );
          })}
        </div>
      </div>
    );
  }

  return (
    <div>
      {label}
      <div
        className="mt-2 flex items-start gap-3 rounded-xl border ghost-border bg-surface-container-lowest px-4 py-3"
        data-testid="onboarding-group-agent"
      >
        <span
          aria-hidden="true"
          className="flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-full bg-primary-container text-sm font-semibold text-on-primary-container"
        >
          {agent.title.charAt(0)}
        </span>
        <span className="min-w-0 flex-1">
          <span className="block text-sm font-semibold text-on-surface truncate">{agent.title}</span>
          <span className="mt-0.5 block text-xs text-on-surface-variant">
            {t(coachCategoryLabelKey(agent.category))}
          </span>
          {agent.description ? (
            <span className="mt-1 block text-sm text-on-surface-variant line-clamp-2">{agent.description}</span>
          ) : null}
        </span>
        <Button
          type="button"
          variant="tertiary"
          onClick={onChange}
          disabled={disabled}
          aria-label={t('onboarding.groupAgentChangeAria')}
          data-testid="onboarding-group-agent-change"
        >
          {t('onboarding.groupAgentChange')}
        </Button>
      </div>
    </div>
  );
}

/**
 * Whether the link goes to the share sheet first: the browser has one (Web
 * Share) and the screen is a phone's — a coarse pointer or below `lg`, the
 * same test the 44px touch targets use (DESIGN.md §8).
 */
function sharesFromPhone(): boolean {
  if (typeof navigator === 'undefined' || typeof navigator.share !== 'function') return false;
  return (
    typeof window.matchMedia === 'function' &&
    window.matchMedia('(pointer: coarse), (max-width: 1023px)').matches
  );
}

/** The invite, one job: share its link. The QR waits behind a link; the seat card tells the truth. */
function InviteShare({
  group,
  link,
  athleteName,
  toCoachHome,
  onDone,
}: {
  group: CoachingGroup;
  link: string;
  athleteName: string;
  toCoachHome: boolean;
  onDone: () => void;
}) {
  const { t } = useTranslation();
  const [qr, setQr] = useState<string | null>(null);
  const [showQr, setShowQr] = useState(false);
  const [copied, setCopied] = useState(false);
  const [onPhone] = useState(sharesFromPhone);

  useEffect(() => {
    if (!showQr) return;
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
  }, [link, showQr]);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(link);
      setCopied(true);
    } catch {
      // The link stays on screen to copy by hand.
    }
  };
  const share = () => void navigator.share({ title: group.name, url: link }).catch(() => {});

  const intro = athleteName
    ? t('onboarding.groupInviteIntroNamed', { name: athleteName, group: group.name, days: ONBOARDING_INVITE_DAYS })
    : t('onboarding.groupInviteLinkIntro', { group: group.name, days: ONBOARDING_INVITE_DAYS });

  return (
    <div className="mx-auto max-w-md">
      <p className="mt-3 text-sm text-on-surface-variant text-center">{intro}</p>

      <p
        className="mt-8 truncate rounded-lg bg-surface-container px-3 py-2.5 font-mono text-sm text-on-surface"
        title={link}
        data-testid="onboarding-group-link"
      >
        {link}
      </p>

      <div className="mt-4 flex flex-col items-center gap-2">
        {onPhone ? (
          <Button variant="primary" onClick={share} className="w-full" data-testid="onboarding-group-share">
            {t('onboarding.groupShareLink')}
          </Button>
        ) : (
          <Button variant="primary" onClick={() => void copy()} className="w-full" data-testid="onboarding-group-copy">
            {copied ? t('onboarding.groupLinkCopied') : t('onboarding.groupCopyLink')}
          </Button>
        )}
        <Button
          variant="tertiary"
          onClick={() => setShowQr((open) => !open)}
          aria-expanded={showQr}
          data-testid="onboarding-group-qr-toggle"
        >
          {showQr ? t('onboarding.groupHideQr') : t('onboarding.groupShowQr')}
        </Button>
      </div>

      {showQr && qr ? (
        <div className="mt-4 flex justify-center">
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

      <SeatCard group={group} />

      <Button
        variant="tertiary"
        onClick={onDone}
        className="mt-8 w-full border ghost-border-strong"
        data-testid="onboarding-group-done"
      >
        {toCoachHome
          ? t('humanCoach.seatInvite.openCoachHome')
          : athleteName
            ? t('onboarding.groupDoneNamed', { name: athleteName })
            : t('onboarding.groupDone')}
      </Button>
    </div>
  );
}

/**
 * Where the coach's seat stands for this group, as the server has it: its
 * coach, a request sent on a date, or nothing asked yet with the quiet
 * request. "Pending" is never said before a request exists.
 */
function SeatCard({ group }: { group: CoachingGroup }) {
  const { t, i18n } = useTranslation();
  const { user } = useAuth();
  const latest = useQuery({
    queryKey: QUERY_KEYS.user.coachAccessRequest(),
    queryFn: () => userApi.getCoachAccessRequest(),
    enabled: !group.coach_user_id,
  });

  if (group.coach_user_id) {
    return (
      <div
        className="mt-8 flex items-start gap-3 rounded-xl border ghost-border bg-surface-container-lowest px-4 py-3"
        data-testid="onboarding-group-seat"
        data-state="coach"
      >
        <CheckDisc />
        <span>
          <span className="block text-sm font-semibold text-on-surface">{t('humanCoach.inviteSeatCoach')}</span>
          <span className="mt-0.5 block text-sm text-on-surface-variant">{t('humanCoach.inviteSeatCoachBody')}</span>
        </span>
      </div>
    );
  }

  // The request half of the seat, read as the room strip and Group info read it.
  const request = latest.data?.request ?? null;
  const pending = request !== null && coachSeatState(request) === 'pending' ? request : null;
  // Who may ask is the room's rule: the coach made this group on the coach
  // path, so they own it; coach access leaves nothing to ask.
  const mayAsk = canRequestCoachAccess({
    group,
    isOwner: true,
    managesRoster: user?.manages_roster === true,
  });
  return (
    <div
      className="mt-8 rounded-xl border ghost-border bg-surface-container-lowest px-4 py-3"
      data-testid="onboarding-group-seat"
      data-state={pending ? 'requested' : 'none'}
    >
      <p className="text-sm font-semibold text-on-surface">{t('humanCoach.inviteSeatPending')}</p>
      {pending ? (
        <p className="mt-1 text-sm text-on-surface-variant">
          {t('humanCoach.inviteSeatRequestedNotice', { date: coachSeatMoment(pending.created_at, i18n.language).date })}
        </p>
      ) : latest.isLoading ? null : (
        <p className="mt-1 text-sm text-on-surface-variant">{t('humanCoach.inviteSeatNotRequested')}</p>
      )}
      <p className="mt-1 text-sm text-on-surface-variant">{t('humanCoach.inviteSeatMeanwhile')}</p>
      {pending || latest.isLoading || !mayAsk ? null : (
        <CoachAccessRequest groupId={group.id} />
      )}
    </div>
  );
}
