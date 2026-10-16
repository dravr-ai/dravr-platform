// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The coach-group onboarding sequence — make the group, open its thread, issue the athlete invite — per client's APIs
// ABOUTME: A retry after a later call failed reuses the group and thread already made rather than making second ones

import { useCallback, useMemo, useRef, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { ONBOARDING_INVITE_DAYS, QUERY_KEYS } from '@pierre/shared-constants';
import type { ChatApi, GroupsApi } from '@pierre/api-client';
import type { CoachingGroup } from '@pierre/shared-types';

/** The APIs the sequence calls, bound by each client. */
export interface CoachGroupSetupApis {
  groupsApi: Pick<GroupsApi, 'createGroup' | 'createInvite'>;
  chatApi: Pick<ChatApi, 'createConversation'>;
}

/** What `useCoachGroupSetup` hands the onboarding step. */
export interface UseCoachGroupSetupResult {
  /**
   * Make the group named `name` with `agentId` as the agent its athletes talk
   * to, open the coach's thread for it, and issue the athlete invite. Never
   * rejects: a failure is read from `failed`, and calling it again resumes
   * from the first call that did not land.
   */
  create: (name: string, agentId: string) => Promise<void>;
  working: boolean;
  /** The last attempt stopped on a failed call; the step says so and offers it again. */
  failed: boolean;
  /** The group once made; its name and agent are fixed from then on. */
  group: CoachingGroup | null;
  /** The coach's thread for the group, where "Go to my group" lands. */
  conversationId: string | null;
  /** The athlete invite's code once issued; each client builds its own link from it. */
  inviteCode: string | null;
}

/**
 * Build `useCoachGroupSetup` over one client's groups and chat APIs.
 *
 * The group is made with the coach as its human coach when they hold coach
 * access (`coach_is_me`); the server decides. The coach's thread for the
 * group is opened with the group's agent, since a group nobody has a thread
 * for is a group the coach cannot find, and the chat list is refreshed so the
 * thread is on it. The invite lasts `ONBOARDING_INVITE_DAYS`.
 */
export function createCoachGroupSetupHook({ groupsApi, chatApi }: CoachGroupSetupApis) {
  return function useCoachGroupSetup(): UseCoachGroupSetupResult {
    const queryClient = useQueryClient();
    const [group, setGroup] = useState<CoachingGroup | null>(null);
    const [conversationId, setConversationId] = useState<string | null>(null);
    const [inviteCode, setInviteCode] = useState<string | null>(null);
    const [working, setWorking] = useState(false);
    const [failed, setFailed] = useState(false);
    // What the calls so far have made, read by a retry in the same render: the
    // state above only reaches the next one.
    const made = useRef<{ group: CoachingGroup | null; conversationId: string | null }>({
      group: null,
      conversationId: null,
    });
    const inFlight = useRef(false);

    const create = useCallback(
      async (name: string, agentId: string) => {
        const trimmed = name.trim();
        if (!trimmed || !agentId || inFlight.current) return;
        inFlight.current = true;
        setWorking(true);
        setFailed(false);
        try {
          const madeGroup =
            made.current.group ??
            (await groupsApi.createGroup({ name: trimmed, agent_id: agentId, coach_is_me: true }));
          made.current.group = madeGroup;
          setGroup(madeGroup);
          const thread =
            made.current.conversationId ??
            (await chatApi.createConversation({ group_id: madeGroup.id, agent_id: madeGroup.agent_id })).id;
          made.current.conversationId = thread;
          setConversationId(thread);
          // The chat list may already be cached: the group's thread must be on it.
          void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() });
          const invite = await groupsApi.createInvite(madeGroup.id, { expires_in_days: ONBOARDING_INVITE_DAYS });
          setInviteCode(invite.code);
        } catch {
          setFailed(true);
        } finally {
          inFlight.current = false;
          setWorking(false);
        }
      },
      [queryClient],
    );

    return useMemo(
      () => ({ create, working, failed, group, conversationId, inviteCode }),
      [create, working, failed, group, conversationId, inviteCode],
    );
  };
}
