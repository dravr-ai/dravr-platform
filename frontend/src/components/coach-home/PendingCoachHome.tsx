// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A coach's Home while their coach access is pending — « Préparer ton coaching »: the seat, the athlete invited, a platform linked
// ABOUTME: The rows tick from the server (GET /api/me/coach-setup); a grant turns Home into the coach Home with no reload (carnet#1529)

import { useEffect, useId, useState, type ReactNode } from 'react';
import { clsx } from 'clsx';
import { Check } from 'lucide-react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from '@pierre/i18n';
import {
  COACH_PLATFORM_PROVIDERS,
  COACH_SEAT_KEYS,
  COACH_SETUP_KEYS,
  ONBOARDING_INVITE_DAYS,
  QUERY_KEYS,
  coachPlatformName,
  coachSeatState,
  defaultGroupAgent,
  groupAgentChoices,
} from '@pierre/shared-constants';
import type { CoachSetupGroup, CoachSetupResponse, CoachSetupRow } from '@pierre/shared-types';
import { COACH_PENDING_INVITE_KEYS, seatLineDay, usableMemberInvite } from '@pierre/ui-logic';
import { coachesApi, providersApi, userApi } from '../../services/api';
import { useAuth } from '../../hooks/useAuth';
import { useIsMobile } from '../../hooks/useBreakpoint';
import { useConversationList } from '../../hooks/useConversationList';
import { useGroupSeatLines, useSeatAfterPlatformLink } from '../../hooks/coachAccessHooks';
import { useCoachGroupSetup, useCreateInvite, useGroupInvites } from '../../hooks/useGroups';
import { avatarShapeClass, avatarSlotClass } from '../chat/avatarSlots';
import CoachAccessRequest from '../CoachAccessRequest';
import ProviderConnectionCards from '../ProviderConnectionCards';
import { Button, EmptyState, Input } from '../ui';
import { CoachHomeSwitch } from './CoachHomeSwitch';
import { inviteLink } from '../../utils/inviteLink';

interface PendingCoachHomeProps {
  /** Dashboard route navigator, `tab[/subview]`. */
  onNavigate: (route: string) => void;
  /**
   * The coach trains too (`coachTrainsToo`): the switch to their own Home
   * leads the header. A coach who does not train has no other Home to go to.
   */
  trains: boolean;
}

/**
 * The Home a coach lands on while their coach access is pending (P1·D,
 * carnet#1529), in place of the athlete's: no Getting started, no provider
 * nudge — only what gets them to their coach Home.
 *
 * « Préparer ton coaching » lists the three steps the server ticks: the coach
 * seat, with the request or its dated state (the one filled button on the
 * page); the athlete invited, which makes the group the onboarding way when
 * there is none, then hands over its link; TrainingPeaks and Intervals.icu as
 * the same plain rows Group info shows (D3) — what connecting one did is the
 * server's, read back from the seat. Under it, the coach's rooms, where the
 * agent already answers.
 *
 * Nothing here flips the Home: once an admin grants the access, the
 * shell's grant refresh patches the session and the Dashboard routes Home to
 * the coach Home; a TrainingPeaks coach account does the same through
 * {@link useSeatAfterPlatformLink}.
 */
export default function PendingCoachHome({ onNavigate, trains }: PendingCoachHomeProps) {
  const { t } = useTranslation();
  const isMobile = useIsMobile();
  const setup = useQuery({
    queryKey: QUERY_KEYS.home.coachSetup(),
    queryFn: () => userApi.getCoachSetup(),
    refetchOnWindowFocus: true,
  });
  const latest = useQuery({
    queryKey: QUERY_KEYS.user.coachAccessRequest(),
    queryFn: () => userApi.getCoachAccessRequest(),
  });
  const providers = useQuery({
    queryKey: QUERY_KEYS.providers.status(),
    queryFn: () => providersApi.getProvidersStatus(),
  });
  useSeatAfterPlatformLink(providers.data?.providers.filter((p) => p.connected).map((p) => p.provider));

  const scope = latest.isSuccess
    ? t(coachSeatState(latest.data.request) === 'pending' ? COACH_SETUP_KEYS.scopePending : COACH_SETUP_KEYS.scope)
    : null;

  let body: ReactNode;
  if (setup.data !== undefined) {
    body = (
      <div className="max-w-xl space-y-8 px-4 py-6 md:px-8">
        <Checklist setup={setup.data} />
        <CoachRooms setup={setup.data} onNavigate={onNavigate} />
      </div>
    );
  } else if (setup.isError) {
    body = (
      <div className="px-4 py-6 md:px-8" data-testid="pending-coach-home-failed">
        <EmptyState action={{ label: t('common.retry'), onClick: () => void setup.refetch() }}>
          {t(COACH_SETUP_KEYS.loadFailed)}
        </EmptyState>
      </div>
    );
  } else {
    body = (
      <div className="flex flex-1 items-center justify-center py-8" role="status" aria-label={t('common.loading')}>
        <div className="pierre-spinner" />
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col bg-surface" data-testid="pending-coach-home">
      <header className="flex min-h-[52px] shrink-0 flex-wrap items-center gap-x-3 gap-y-2 border-b ghost-border px-4 py-2">
        {trains ? (
          <CoachHomeSwitch scope="athletes" onChange={() => onNavigate('home/me')} stretch={isMobile} />
        ) : (
          <h1 className="font-display text-lg font-semibold tracking-tight text-on-surface">
            {t('coachHome.switch.athletes')}
          </h1>
        )}
        {scope !== null && (
          <span className="text-xs text-on-surface-variant" data-testid="pending-coach-scope">
            {scope}
          </span>
        )}
      </header>
      <div className="min-h-0 flex-1 overflow-y-auto">{body}</div>
    </div>
  );
}

/** « Préparer ton coaching »: the server's rows, in its order, with its count. */
function Checklist({ setup }: { setup: CoachSetupResponse }) {
  const { t } = useTranslation();
  const titleId = useId();
  const rows = [...setup.rows].sort((a, b) => a.order - b.order);
  return (
    <section aria-labelledby={titleId} className="space-y-2.5" data-testid="coach-setup">
      <div className="flex items-baseline gap-2">
        <h2 id={titleId} className="font-display text-base font-semibold tracking-tight text-on-surface">
          {t(COACH_SETUP_KEYS.title)}
        </h2>
        <span aria-hidden="true" className="font-mono text-xs text-outline" data-testid="coach-setup-count">
          {t(COACH_SETUP_KEYS.count, { done: setup.done_count, total: setup.total })}
        </span>
        <span className="sr-only">{t(COACH_SETUP_KEYS.countAria, { done: setup.done_count, total: setup.total })}</span>
      </div>
      <ol className="space-y-1">
        {rows.map((row) => (
          <SetupRow key={row.id} row={row} setup={setup} />
        ))}
      </ol>
    </section>
  );
}

/** One row: its marker, its title (« · fait » once done), and what it holds. */
function SetupRow({ row, setup }: { row: CoachSetupRow; setup: CoachSetupResponse }) {
  const { t } = useTranslation();
  const open = row.current && !row.done;
  let title: string;
  let content: ReactNode;
  switch (row.id) {
    case 'seat':
      title = t(COACH_SEAT_KEYS.panelTitle);
      content = row.done ? null : <CoachAccessRequest variant="row" />;
      break;
    case 'invite':
      title = t(COACH_SETUP_KEYS.inviteTitle);
      content = <InviteContent setup={setup} done={row.done} />;
      break;
    case 'platform':
      title = t(COACH_SETUP_KEYS.platformTitle);
      content = <PlatformContent setup={setup} done={row.done} />;
      break;
  }
  return (
    <li
      data-testid={`coach-setup-row-${row.id}`}
      data-state={row.done ? 'done' : open ? 'current' : 'open'}
      aria-current={open ? 'step' : undefined}
      className={clsx(
        'flex gap-3 rounded-xl border px-4',
        open ? 'ghost-border bg-surface-container-lowest py-3.5' : 'border-transparent py-3',
      )}
    >
      <span
        aria-hidden="true"
        className={clsx(
          'mt-px flex h-[22px] w-[22px] shrink-0 items-center justify-center rounded-full',
          row.done && 'bg-primary text-on-primary',
          open && 'border-2 border-primary',
          !row.done && !open && 'border-[1.5px] border-outline',
        )}
      >
        {row.done && <Check className="h-3 w-3" strokeWidth={3} />}
      </span>
      <div className="min-w-0 flex-1 space-y-1">
        <p className="text-sm font-semibold text-on-surface">
          {title}
          {row.done && <span className="font-normal text-on-surface-variant">{` · ${t(COACH_SETUP_KEYS.done)}`}</span>}
        </p>
        {content}
      </div>
    </li>
  );
}

/**
 * The invite row: who joined and when, once someone has; else the group's
 * link to send; else, with no group yet, the group made the onboarding way —
 * named after the athlete, on the endurance generalist (D8), with its thread
 * and its link.
 */
function InviteContent({ setup, done }: { setup: CoachSetupResponse; done: boolean }) {
  const { t, language } = useTranslation();
  if (done && setup.athlete) {
    const date = seatLineDay(setup.athlete.joined_at, language);
    return (
      <p className="text-sm text-on-surface-variant" data-testid="coach-setup-athlete-joined">
        {setup.athlete.name
          ? t(COACH_SETUP_KEYS.inviteDone, { name: setup.athlete.name, date })
          : t(COACH_SETUP_KEYS.inviteDoneNoName, { date })}
      </p>
    );
  }
  const group = setup.groups[0];
  return group ? <InviteLink group={group} /> : <CreateGroup />;
}

/** The coach's first group, made as onboarding makes it, from the athlete's name. */
function CreateGroup() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [athlete, setAthlete] = useState('');
  const agents = useQuery({ queryKey: QUERY_KEYS.coaches.list(), queryFn: () => coachesApi.list() });
  const agent = defaultGroupAgent(groupAgentChoices(agents.data?.agents ?? []));
  const { create, working, failed, group } = useCoachGroupSetup();
  // The checklist reads the group the server now holds, and its link.
  useEffect(() => {
    if (group) void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.home.coachSetup() });
  }, [group, queryClient]);
  const name = athlete.trim();

  return (
    <>
      <p className="text-sm text-on-surface-variant">{t(COACH_SETUP_KEYS.groupIntro)}</p>
      <form
        className="flex flex-col gap-2 pt-2 sm:flex-row sm:items-end"
        onSubmit={(event) => {
          event.preventDefault();
          if (name !== '' && agent) void create(name, agent.id);
        }}
      >
        <div className="sm:flex-1">
          <Input
            label={t(COACH_SETUP_KEYS.athleteLabel)}
            placeholder={t(COACH_SETUP_KEYS.athletePlaceholder)}
            value={athlete}
            maxLength={60}
            disabled={working || group !== null}
            onChange={(event) => setAthlete(event.target.value)}
            data-testid="coach-setup-athlete"
          />
        </div>
        <Button
          type="submit"
          variant="secondary"
          disabled={working || group !== null || name === '' || !agent}
          data-testid="coach-setup-create"
        >
          {working ? t(COACH_SETUP_KEYS.creating) : t(COACH_SETUP_KEYS.create)}
        </Button>
      </form>
      {failed && (
        <p className="text-sm text-error" role="alert">
          {t(COACH_SETUP_KEYS.createFailed)}
        </p>
      )}
    </>
  );
}

/**
 * The group's link to send: an invite that still works says so, neutrally
 * (D19); with none, a new one is issued the onboarding way on the first copy.
 */
function InviteLink({ group }: { group: CoachSetupGroup }) {
  const { t } = useTranslation();
  const { invites, isLoading } = useGroupInvites(group.id);
  const { createInvite, isPending } = useCreateInvite(group.id);
  const [copied, setCopied] = useState(false);
  const [copyFailed, setCopyFailed] = useState(false);
  const invite = usableMemberInvite(invites, new Date());

  const copy = async () => {
    setCopyFailed(false);
    try {
      const code = invite?.code ?? (await createInvite({ expires_in_days: ONBOARDING_INVITE_DAYS })).code;
      await navigator.clipboard.writeText(inviteLink(code));
      setCopied(true);
    } catch {
      setCopyFailed(true);
    }
  };

  if (isLoading) return null;
  return (
    <>
      {invite ? (
        <p className="text-sm text-on-surface-variant" data-testid="coach-setup-invite-status">
          <span className="font-medium text-on-surface">{group.name}</span>
          {` · ${t(COACH_PENDING_INVITE_KEYS.status)}`}
        </p>
      ) : (
        <p className="text-sm text-on-surface-variant">
          {t(COACH_SETUP_KEYS.linkIntro, { group: group.name, days: ONBOARDING_INVITE_DAYS })}
        </p>
      )}
      <div className="-ml-1.5 pt-0.5">
        <button
          type="button"
          onClick={() => void copy()}
          disabled={isPending}
          className="btn-base btn-tertiary touch-target min-h-[32px] px-1.5 disabled:opacity-50"
          data-testid="coach-setup-copy-link"
        >
          {copied
            ? t(COACH_SETUP_KEYS.linkCopied)
            : invite
              ? t(COACH_PENDING_INVITE_KEYS.copyAgain)
              : t(COACH_SETUP_KEYS.copyLink)}
        </button>
      </div>
      {copyFailed && (
        <p className="text-sm text-error" role="alert">
          {t(COACH_PENDING_INVITE_KEYS.copyFailed)}
        </p>
      )}
    </>
  );
}

/**
 * The coaching-platform row: TrainingPeaks and Intervals.icu as the plain,
 * identical rows of Group info's seat block (D3); once one is linked, which.
 */
function PlatformContent({ setup, done }: { setup: CoachSetupResponse; done: boolean }) {
  const { t } = useTranslation();
  if (done && setup.platform) {
    return (
      <p className="text-sm text-on-surface-variant" data-testid="coach-setup-platform-linked">
        {t(COACH_SETUP_KEYS.platformDone, { provider: coachPlatformName(setup.platform) })}
      </p>
    );
  }
  return (
    <>
      <p className="text-sm text-on-surface-variant">{t(COACH_SETUP_KEYS.platformBody)}</p>
      <div className="pt-1" data-testid="coach-setup-platforms">
        <ProviderConnectionCards onlyProviders={COACH_PLATFORM_PROVIDERS} plain coachKey />
      </div>
    </>
  );
}

/**
 * The coach's rooms, as the Groups list draws them — with the coach-seat line
 * in place of the last words while the seat moves — and that the agent
 * already answers there.
 */
function CoachRooms({ setup, onNavigate }: { setup: CoachSetupResponse; onNavigate: (route: string) => void }) {
  const { t } = useTranslation();
  const titleId = useId();
  const { user } = useAuth();
  const { rows } = useConversationList();
  const groupIds = new Set(setup.groups.map((group) => group.id));
  const rooms = rows.filter((row) => row.kind === 'group' && row.groupId !== null && groupIds.has(row.groupId));
  const seatLines = useGroupSeatLines(rooms, user ?? null);
  if (rooms.length === 0) return null;

  return (
    <section aria-labelledby={titleId} className="space-y-2.5" data-testid="coach-setup-rooms">
      <h2 id={titleId} className="font-display text-base font-semibold tracking-tight text-on-surface">
        {t(rooms.length === 1 ? COACH_SETUP_KEYS.groupsTitleOne : COACH_SETUP_KEYS.groupsTitleN)}
      </h2>
      <ul className="space-y-1.5">
        {rooms.map((row) => {
          const line = seatLines.get(row.id) ?? row.preview;
          return (
            <li key={row.id}>
              <button
                type="button"
                onClick={() => onNavigate(`chat/${encodeURIComponent(row.id)}`)}
                className="flex w-full items-center gap-3 rounded-xl border ghost-border bg-surface-container-lowest px-3.5 py-3 text-left transition-colors hover:bg-surface-container-low focus-ring"
                data-testid={`coach-setup-room-${row.groupId}`}
              >
                <span
                  aria-hidden="true"
                  className={clsx(
                    'flex h-10 w-10 shrink-0 items-center justify-center text-sm font-semibold',
                    avatarSlotClass(row.avatarSlot),
                    avatarShapeClass('square'),
                  )}
                >
                  {row.initials}
                </span>
                <span className="min-w-0 flex-1">
                  <span className="flex items-baseline gap-2">
                    <span className="min-w-0 flex-1 truncate text-sm font-semibold text-on-surface">{row.title}</span>
                    <span className="shrink-0 text-xs text-on-surface-variant">{row.timestamp}</span>
                  </span>
                  {line !== '' && <span className="block truncate text-sm text-on-surface-variant">{line}</span>}
                </span>
              </button>
            </li>
          );
        })}
      </ul>
      <p className="text-xs text-on-surface-variant">{t(COACH_SETUP_KEYS.groupsNote)}</p>
    </section>
  );
}
