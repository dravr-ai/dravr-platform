// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: One athlete as the group's coach meets them in Group info — who, whether they share, and the way into their plan
// ABOUTME: Coach Home's pattern and weight: the plan thread filled for a group's one athlete, the read-only plan in ink beside it

import type { ReactNode } from 'react';
import { ChevronRight } from 'lucide-react';
import type { GroupMember } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import { Button } from '../ui';
import RoleWord from './RoleWord';
import { PersonAvatar, RowBox, SharingState } from './GroupInfoParts';

interface CoachAthleteCardProps {
  member: GroupMember;
  /**
   * The group's one athlete: their plan thread is the drawer's one filled
   * button, as it is on coach Home. With several, every card is ink.
   */
  primary: boolean;
  /** Open the coach's plan thread for this athlete. */
  onWorkOnPlan?: (member: GroupMember) => void;
  /** This athlete's plan thread is being opened. */
  opening: boolean;
  /** Some plan thread is being opened, so none can be started twice. */
  busy: boolean;
  /** Read the athlete's live plan without opening the thread (carnet#1244). */
  onViewPlan?: (member: GroupMember) => void;
  /** Open this athlete on coach Home, for a coach who has one. */
  onOpenCoachHome?: (member: GroupMember) => void;
  /** The `⋯` menu, for an owner or admin who may change their role or remove them. */
  menu?: ReactNode;
}

/**
 * The coach's job in a group is the plan, so its actions are words, never
 * bare icons: the visible label leads each accessible name (WCAG 2.5.3) with
 * the athlete's name after it, so a screen reader can tell two athletes'
 * buttons apart.
 */
export default function CoachAthleteCard({
  member,
  primary,
  onWorkOnPlan,
  opening,
  busy,
  onViewPlan,
  onOpenCoachHome,
  menu,
}: CoachAthleteCardProps) {
  const { t } = useTranslation();
  const name = member.display_name ?? t('groups.memberFallback');

  return (
    <RowBox data-testid={`coach-athlete-${member.user_id}`}>
      <div className="space-y-3 p-4">
        <div className="flex items-center gap-3">
          <PersonAvatar userId={member.user_id} name={name} />
          <div className="min-w-0 flex-1">
            <p className="truncate text-base font-semibold text-on-surface">{name}</p>
            <span
              className="flex flex-wrap items-center gap-x-1.5 text-xs text-on-surface-variant"
              data-testid={`member-detail-${member.user_id}`}
            >
              <RoleWord kind={member.role} />
              <span aria-hidden="true">·</span>
              <SharingState on={member.peer_sharing_consent} onLabel={t('groups.sharingOn')} offLabel={t('groups.sharingOff')} />
            </span>
          </div>
          {menu}
        </div>
        {(onWorkOnPlan || onViewPlan) && (
          <div className="flex flex-wrap gap-2">
            {onWorkOnPlan && (
              <Button
                variant={primary ? 'primary' : 'outline'}
                onClick={() => onWorkOnPlan(member)}
                loading={opening}
                disabled={busy}
                data-testid={`member-work-on-plan-${member.user_id}`}
              >
                {t('coachHome.action.openPlanThread')}
                <span className="sr-only"> · {name}</span>
              </Button>
            )}
            {onViewPlan && (
              <Button variant="outline" onClick={() => onViewPlan(member)} data-testid={`member-view-plan-${member.user_id}`}>
                {t('humanCoach.athleteCard.viewPlan')}
                <span className="sr-only"> · {name}</span>
              </Button>
            )}
          </div>
        )}
      </div>
      {onOpenCoachHome && (
        <button
          type="button"
          onClick={() => onOpenCoachHome(member)}
          data-testid={`member-coach-home-${member.user_id}`}
          className="flex h-10 w-full items-center justify-between rounded-b-xl border-t ghost-border-faint px-4 text-left text-sm font-medium text-primary transition-colors hover:bg-surface-container-low focus-ring"
        >
          <span>
            {t('humanCoach.athleteCard.seeOnCoachHome')}
            <span className="sr-only"> · {name}</span>
          </span>
          <ChevronRight className="h-4 w-4" aria-hidden="true" />
        </button>
      )}
    </RowBox>
  );
}
