// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The two who run a coaching group without a membership row — its AI agent and its human coach — as Group info rows
// ABOUTME: The coach slot names the owner offered the seat, or says plainly that the group has no coach yet

import type { ReactNode } from 'react';
import { UserRound } from 'lucide-react';
import type { CoachingGroup } from '@pierre/shared-types';
import { COACH_SEAT_KEYS, MENTION_PREFIX, type CoachSeatState } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import RoleWord from './RoleWord';
import { AgentAvatar, PersonAvatar, PersonRow } from './GroupInfoParts';

type LeadsGroup = Pick<CoachingGroup, 'id' | 'agent_title' | 'agent_handle' | 'coach_user_id' | 'coach_display_name'>;

/** A person slot with no one to draw yet: the glyph alone, so the row's words are its whole text. */
function EmptySlotAvatar() {
  return (
    <span
      aria-hidden="true"
      className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full border border-dashed border-outline-variant text-on-surface-variant"
    >
      <UserRound className="h-4 w-4" />
    </span>
  );
}

/**
 * The agent that answers in the group's room. The server names it as the
 * caller reads it, which also covers an agent from another tenant that the
 * caller's own agent list cannot see.
 */
export function AgentRow({ group }: { group: LeadsGroup }) {
  const { t } = useTranslation();
  return (
    <PersonRow
      data-testid="group-info-agent"
      avatar={<AgentAvatar />}
      title={
        <>
          <span>{group.agent_title ?? t('app.aiAgent')}</span>
          {group.agent_handle && (
            <span className="ml-2 font-mono text-xs font-normal text-on-surface-variant" data-testid="group-info-agent-handle">
              {`${MENTION_PREFIX}${group.agent_handle}`}
            </span>
          )}
        </>
      }
      detail={<RoleWord kind="agent" />}
    />
  );
}

interface CoachRowProps {
  group: LeadsGroup;
  /**
   * Where the viewer's coach seat stands, when they are offered it
   * (`canRequestCoachAccess`): the slot then names them, not yet the coach
   * or waiting on the team, instead of telling the person who will be the
   * coach that the group has none. The request itself is Group info's first
   * block.
   */
  seatState?: CoachSeatState | null;
  /** The coach row's `⋯` menu, for an admin who may detach the coach. */
  menu?: ReactNode;
}

/**
 * The group's human coach, the viewer's own seat while they wait for it, or
 * the plain fact that there is no coach. The coach never reads this row:
 * their drawer leads with their athletes instead.
 */
export function CoachRow({ group, seatState = null, menu }: CoachRowProps) {
  const { t } = useTranslation();

  if (group.coach_user_id) {
    const name = group.coach_display_name ?? t('humanCoach.attached');
    return (
      <PersonRow
        data-testid="group-info-coach-row"
        avatar={<PersonAvatar userId={group.coach_user_id} name={name} />}
        title={<span data-testid="group-info-coach-name">{name}</span>}
        detail={<RoleWord kind="coach" />}
        trailing={menu}
      />
    );
  }

  if (seatState) {
    return (
      <PersonRow
        data-testid="group-info-coach-seat"
        data-state={seatState}
        avatar={<EmptySlotAvatar />}
        title={
          <span className={seatState === 'pending' ? 'text-on-warning-container' : 'font-normal text-on-surface-variant'}>
            {seatState === 'pending' ? t(COACH_SEAT_KEYS.rosterPending) : t(COACH_SEAT_KEYS.rosterNotYet)}
          </span>
        }
      />
    );
  }

  return (
    <PersonRow
      data-testid="group-info-no-coach"
      avatar={<EmptySlotAvatar />}
      title={<span className="font-normal text-on-surface-variant">{t('humanCoach.noneMember')}</span>}
    />
  );
}
