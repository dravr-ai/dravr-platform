// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Coach Home before anyone joined — each group nobody has joined yet whose invite still works, as one true row: invited, not joined
// ABOUTME: Copy the link again or write in the group; with no such group the empty state stands, never an "invited" row for a dead link

import { useState } from 'react';
import { useQueries } from '@tanstack/react-query';
import type { CoachRosterGroup, GroupInvite } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import { formatDate, formatMessageTime, GROUP_ROOM_KEYS } from '@pierre/chat-utils';
import { COACH_PENDING_INVITE_KEYS, groupsAwaitingAthlete, usableMemberInvite } from '@pierre/ui-logic';
import { groupsApi } from '../../services/api';
import { QUERY_KEYS } from '../../constants/queryKeys';
import { useOpenGroupRoom } from '../../hooks/useCoachPlan';
import { Button, EmptyState, useErrorToast } from '../ui';
import { AthleteAvatar } from './parts';

/** A group nobody has joined, with the invite its athlete can still join with. */
interface PendingInvite {
  group: CoachRosterGroup;
  invite: GroupInvite;
}

/**
 * What a coach whose groups are all still empty sees on their Home: À
 * traiter, one row per group whose athlete has not joined yet — the group,
 * « invitation sent, not joined yet », the time it was sent, why it is there,
 * and the two things to do about it. The invites are read per empty group
 * (a coach starts with one), so a row is shown only while its link still
 * works.
 */
export function CoachPendingInvites({
  groups,
  onNavigate,
}: {
  groups: readonly CoachRosterGroup[];
  onNavigate: (route: string) => void;
}) {
  const { t } = useTranslation();
  const awaiting = groupsAwaitingAthlete(groups);
  const reads = useQueries({
    queries: awaiting.map((group) => ({
      queryKey: QUERY_KEYS.groups.invites(group.id),
      queryFn: () => groupsApi.listInvites(group.id),
    })),
  });
  const now = new Date();
  const pending: PendingInvite[] = awaiting.flatMap((group, i) => {
    const invite = usableMemberInvite(reads[i]?.data?.invites ?? [], now);
    return invite ? [{ group, invite }] : [];
  });

  if (reads.some((read) => read.isLoading)) {
    return (
      <div className="flex flex-1 items-center justify-center" role="status" aria-label={t('coachHome.loading')}>
        <div className="pierre-spinner" />
      </div>
    );
  }
  if (pending.length === 0) {
    return (
      <div className="px-6 py-6">
        <EmptyState data-testid="coach-home-empty">{t('coachHome.noAthletes')}</EmptyState>
      </div>
    );
  }

  return (
    <div className="px-3 pb-6 pt-4" data-testid="coach-pending-invites">
      <div className="flex items-baseline gap-2.5 px-2 pb-3">
        <h1 className="m-0 font-display text-xl font-semibold tracking-tight text-on-surface">
          {t('coachHome.queue.title')}
        </h1>
        <span className="font-mono text-xs text-on-surface-variant">{pending.length}</span>
      </div>
      <section aria-label={t('coachHome.queue.today')}>
        <div className="px-2 pb-1.5 pt-2.5 text-xs font-medium text-on-surface-variant">{t('coachHome.queue.today')}</div>
        {pending.map((entry) => (
          <PendingInviteRow key={entry.group.id} entry={entry} onNavigate={onNavigate} />
        ))}
      </section>
      <p className="max-w-prose px-2 pt-4 text-sm text-on-surface-variant">{t(COACH_PENDING_INVITE_KEYS.after)}</p>
    </div>
  );
}

function PendingInviteRow({ entry, onNavigate }: { entry: PendingInvite; onNavigate: (route: string) => void }) {
  const { t, language } = useTranslation();
  const { openRoom, pendingRoomAthleteId } = useOpenGroupRoom();
  const showError = useErrorToast();
  const [copied, setCopied] = useState(false);
  const { group, invite } = entry;
  const roomKey = `invite:${group.id}`;
  const link = `${window.location.origin}/groups/join/${encodeURIComponent(invite.code)}`;

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(link);
      setCopied(true);
    } catch {
      showError(t(COACH_PENDING_INVITE_KEYS.copyFailed));
    }
  };
  const writeToGroup = () => {
    openRoom(roomKey, group.id)?.then(
      (room) => onNavigate(`chat/${encodeURIComponent(room.id)}`),
      () => showError(t(GROUP_ROOM_KEYS.openError)),
    );
  };
  const opening = pendingRoomAthleteId === roomKey;

  return (
    <article className="rounded-[10px] bg-surface-container-low px-2.5 py-3" data-testid={`coach-pending-invite-${group.id}`}>
      <div className="flex items-center gap-3">
        <AthleteAvatar userId={group.id} name={group.name} muted />
        <span className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="truncate text-sm font-semibold text-on-surface">{group.name}</span>
          <span className="truncate text-sm text-on-surface-variant" data-testid="coach-pending-invite-status">
            {t(COACH_PENDING_INVITE_KEYS.status)}
          </span>
        </span>
        <span className="shrink-0 font-mono text-xs text-on-surface-variant">
          {formatMessageTime(invite.created_at, language)}
        </span>
      </div>
      <div className="mt-3 pl-12">
        <p className="text-xs font-medium text-on-surface-variant">{t('coachHome.detail.why')}</p>
        <p className="mt-0.5 text-sm text-on-surface">
          {invite.expires_at
            ? t(COACH_PENDING_INVITE_KEYS.why, { date: formatDate(invite.expires_at, language) })
            : t(COACH_PENDING_INVITE_KEYS.whyNoExpiry)}
        </p>
        <div className="mt-3 flex flex-wrap gap-2">
          <Button size="sm" variant="primary" onClick={() => void copy()} data-testid="coach-pending-invite-copy">
            {copied ? t(COACH_PENDING_INVITE_KEYS.copied) : t(COACH_PENDING_INVITE_KEYS.copyAgain)}
          </Button>
          <Button
            size="sm"
            variant="tertiary"
            onClick={writeToGroup}
            disabled={opening}
            data-testid="coach-pending-invite-write"
          >
            {opening ? t('coachHome.action.openingGroup') : t('coachHome.action.writeToGroup')}
          </Button>
        </div>
      </div>
    </article>
  );
}
