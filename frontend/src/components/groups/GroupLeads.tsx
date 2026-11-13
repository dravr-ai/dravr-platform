// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The two who run a coaching group without a membership row — its AI agent and its human coach
// ABOUTME: Drawn as roster rows above the members, each named and badged, so every viewer can tell who is who

import { clsx } from 'clsx';
import type { CoachingGroup } from '@pierre/shared-types';
import { COACH_SEAT_KEYS, MENTION_PREFIX, type CoachSeatState } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import RoleBadge from './RoleBadge';

interface GroupLeadsProps {
  group: Pick<
    CoachingGroup,
    'id' | 'agent_title' | 'agent_handle' | 'coach_user_id' | 'coach_display_name'
  >;
  /** The viewer is the group's human coach. */
  viewerIsCoach: boolean;
  /**
   * Where the viewer's coach seat stands, when they are offered it
   * (`canRequestCoachAccess`): the coach slot then names them, not yet the
   * coach or waiting on the team, instead of telling the person who will be
   * the coach that the group has none. The request itself is Group info's
   * first block.
   */
  seatState?: CoachSeatState | null;
}

const ROW = 'flex items-center justify-between gap-3 border-b ghost-border py-3 px-4';

/**
 * The agent that answers in the group's chat and the human coach who oversees
 * it. Neither holds a membership row, so the member list alone never showed
 * them; the server names both as the caller reads them, which also covers an
 * agent from another tenant that the caller's own agent list cannot see.
 */
export default function GroupLeads({
  group,
  viewerIsCoach,
  seatState = null,
}: GroupLeadsProps) {
  const { t } = useTranslation();

  return (
    <ul className="text-sm" data-testid="group-info-leads">
      <li className={ROW} data-testid="group-info-agent">
        <span className="min-w-0 truncate">
          <span className="font-medium text-on-surface">{group.agent_title ?? t('app.aiAgent')}</span>
          {group.agent_handle && (
            <span className="ml-2 text-xs text-outline" data-testid="group-info-agent-handle">
              {`${MENTION_PREFIX}${group.agent_handle}`}
            </span>
          )}
        </span>
        <RoleBadge kind="agent" />
      </li>
      {group.coach_user_id ? (
        <li className={clsx(ROW, viewerIsCoach && 'bg-primary/5')} data-testid="group-info-coach-row">
          <span className="min-w-0 truncate">
            <span className="font-medium text-on-surface">
              {group.coach_display_name ?? t('humanCoach.attached')}
            </span>
            {viewerIsCoach && <span className="ml-2 text-xs text-outline">{t('groups.youSuffix')}</span>}
          </span>
          <RoleBadge kind="coach" />
        </li>
      ) : seatState ? (
        <li className={ROW} data-testid="group-info-coach-seat" data-state={seatState}>
          <span
            className={clsx(
              'min-w-0 truncate',
              seatState === 'pending' ? 'font-medium text-on-warning-container' : 'text-on-surface-variant',
            )}
          >
            {seatState === 'pending' ? t(COACH_SEAT_KEYS.rosterPending) : t(COACH_SEAT_KEYS.rosterNotYet)}
          </span>
        </li>
      ) : (
        <li className="border-b ghost-border py-3 px-4 text-outline" data-testid="group-info-no-coach">
          {t('humanCoach.noneMember')}
        </li>
      )}
    </ul>
  );
}
