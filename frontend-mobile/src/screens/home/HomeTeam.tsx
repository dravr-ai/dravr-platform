// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Home's Your team as one row — the team's avatars, everyone's names lead first, and Manage, which opens Discover
// ABOUTME: The team is the agent list's is_assigned rows, the lead the agent the personal thread is bound to, as on the web (carnet#1528)

import React, { useMemo } from 'react';
import { Pressable, Text, View } from 'react-native';
import { useQuery } from '@tanstack/react-query';
import { useTranslation } from '@pierre/i18n';
import { QUERY_KEYS } from '@pierre/shared-constants';
import { avatarSlot, formatList, initialsFor } from '@pierre/chat-utils';
import { orderTeam, teamInOrder } from '@pierre/ui-logic';
import { chatApi, coachesApi } from '../../services/api';
import { InitialsAvatar } from '../../components/ui';

/** The agent list changes when the athlete adds or removes one, which invalidates it; the cache stays warm. */
const AGENT_LIST_STALE_MS = 5 * 60_000;

/** The most avatars the row stacks; the names line still names everyone. */
const ROW_AVATARS = 3;

/** Diameter of a stacked avatar, in points. */
const AVATAR_SIZE = 28;

interface HomeTeamProps {
  /** Open Discover, where an agent joins or leaves the team. */
  onManage: () => void;
}

/**
 * Your team, folded to one row (carnet#1528, P5·C): the first three avatars,
 * overlapping on a ring of the page's own surface, « Your team » over
 * everyone's names, and « Manage ». Nothing is drawn until the list answers,
 * nor for an athlete with no team: the phone's Home has no team picker to
 * offer them (the limitation `HomeScreen` registers).
 */
export function HomeTeam({ onManage }: HomeTeamProps) {
  const { t, i18n } = useTranslation();
  const agents = useQuery({
    queryKey: QUERY_KEYS.coaches.list(),
    queryFn: () => coachesApi.list(),
    staleTime: AGENT_LIST_STALE_MS,
  });
  const personal = useQuery({
    queryKey: QUERY_KEYS.chat.personal(),
    queryFn: () => chatApi.getPersonalConversation(),
  });
  const team = useMemo(
    () => teamInOrder(orderTeam(agents.data?.agents ?? [], personal.data?.agent_id ?? null)),
    [agents.data, personal.data],
  );

  if (agents.data === undefined || personal.isLoading || team.length === 0) return null;

  return (
    <View className="mx-4 flex-row items-center gap-3" testID="home-team">
      <View className="flex-row items-center" accessibilityElementsHidden importantForAccessibility="no-hide-descendants">
        {team.slice(0, ROW_AVATARS).map((agent, index) => (
          <View
            key={agent.id}
            className={`rounded-full border-2 border-background-primary${index === 0 ? '' : ' -ml-2'}`}
          >
            <InitialsAvatar
              initials={initialsFor(agent.title)}
              slot={avatarSlot({ id: agent.id, agent_id: agent.id, group_id: null, subject_user_id: null })}
              size={AVATAR_SIZE}
            />
          </View>
        ))}
      </View>
      <View className="min-w-0 flex-1">
        <Text accessibilityRole="header" className="text-sm font-semibold text-text-primary">
          {t('home.team.title')}
        </Text>
        <Text numberOfLines={1} className="text-xs text-text-secondary" testID="home-team-names">
          {formatList(
            team.map((agent) => agent.title),
            i18n.language,
          )}
        </Text>
      </View>
      <Pressable
        onPress={onManage}
        accessibilityRole="button"
        accessibilityLabel={t('home.team.manageAria')}
        className="min-h-11 justify-center px-1"
        testID="home-team-manage"
      >
        <Text className="text-sm font-medium text-primary">{t('home.team.manage')}</Text>
      </Pressable>
    </View>
  );
}
