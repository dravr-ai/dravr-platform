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
import { useGroup, useGroupMembers } from './useGroups';
import { useOnboardingStatus } from './useOnboardingStatus';

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
 * group). Each read waits on the one before it — the members only for a
 * coachless group seen without coach access, the onboarding answer only for
 * its owner, the request only once the seat is offered — and shares its query
 * key with Group info.
 */
export function useCoachSeat(groupId: string | null | undefined): CoachSeat {
  const { user } = useAuth();
  const id = groupId ?? '';
  const { group } = useGroup(id);
  const managesRoster = user?.manages_roster === true;
  // Whether the rule could still offer the seat, before the two reads that
  // say whether the viewer owns the group and coaches others.
  const mayOffer = canRequestCoachAccess({ group, isOwner: true, managesRoster, coachesOthers: true });
  const { members } = useGroupMembers(mayOffer ? id : '');
  const isOwner = members.some((m) => m.user_id === user?.id && m.role === 'owner');
  const { data: onboarding } = useOnboardingStatus(mayOffer && isOwner);
  const offered = canRequestCoachAccess({
    group,
    isOwner,
    managesRoster,
    coachesOthers: onboarding?.coaches_others === true,
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
