// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Whether the viewer is offered a group's coach seat, and where their coach-access request stands (mobile)
// ABOUTME: Mirrors the web hook: the room strip, the header line and the Group info block read this one source

import { useQuery } from '@tanstack/react-query';
import {
  QUERY_KEYS,
  canRequestCoachAccess,
  coachSeatState,
  type CoachSeatState,
} from '@pierre/shared-constants';
import type { CoachAccessRequest } from '@pierre/shared-types';
import { userApi } from '../services/api';
import { useAuth } from '../contexts/AuthContext';
import { useGroup } from './useGroups';

export interface CoachSeat {
  /** The viewer is offered this group's coach seat (`canRequestCoachAccess`). */
  offered: boolean;
  /**
   * Where the seat stands once the latest request is read; `null` when the
   * seat is not offered or still loading, so nothing flashes a wrong state.
   */
  state: CoachSeatState | null;
  /** The latest request, when there is one. */
  request: CoachAccessRequest | null;
}

/**
 * The coach seat for the group behind `groupId` (`null` for a thread with no
 * group). The group says who owns it and whether it has a coach; the request
 * is read only once the seat is offered. What the viewer answered to "I coach
 * others" plays no part (carnet#1522): the owner of a coachless group without
 * coach access is offered the seat whatever they answered. The reads share
 * their query keys with Group info.
 */
export function useCoachSeat(groupId: string | null | undefined): CoachSeat {
  const { user } = useAuth();
  const { group } = useGroup(groupId ?? '');
  const offered = canRequestCoachAccess({
    group,
    isOwner: !!group && !!user && group.owner_id === user.id,
    managesRoster: user?.manages_roster === true,
  });
  const latest = useQuery({
    queryKey: QUERY_KEYS.user.coachAccessRequest(),
    queryFn: () => userApi.getCoachAccessRequest(),
    enabled: offered,
  });
  const request = latest.data?.request ?? null;
  // A failed read still offers the request: the server keeps one pending
  // request per account, so asking again is harmless.
  const settled = latest.isSuccess || latest.isError;
  return {
    offered,
    state: offered && settled ? coachSeatState(request) : null,
    request,
  };
}
