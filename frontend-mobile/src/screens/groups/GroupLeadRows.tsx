// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Who runs a coaching group — the AI agent that answers in its chat and the human coach who oversees it — as roster rows
// ABOUTME: Drawn above the member rows; the names are the group's own, resolved by the server, so every viewer reads the same ones

import React from 'react';
import { View, TouchableOpacity, ActivityIndicator } from 'react-native';
import { Feather } from '@expo/vector-icons';
import { avatarSlot, initialsFor } from '@pierre/chat-utils';
import { MENTION_PREFIX } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { useThemeColors } from '../../constants/theme';
import { InitialsAvatar } from '../../components/ui/InitialsAvatar';
import { RosterRow } from './RosterRow';
import type { CoachingGroup } from '../../types';

export interface GroupLeadRowsProps {
  group: CoachingGroup;
  /** The caller is the group's attached human coach. */
  viewerIsCoach: boolean;
  /** The caller owns or administers the group: may detach the coach, and may invite one. */
  isAdmin: boolean;
  onRemoveCoach: () => void;
  isRemovingCoach: boolean;
  /** No member row follows: the coach line draws no hairline under itself. */
  last?: boolean;
}

/**
 * The AI agent, then the human coach, as the first lines of the roster.
 *
 * The agent's title and handle come on the group itself, resolved against the
 * group's tenant: a member who joined from another tenant has no such agent
 * on their own coach list, and resolving it there named it "AI agent" for
 * them alone. The coach holds no membership row, so this is the only line
 * that names them.
 */
export function GroupLeadRows({
  group,
  viewerIsCoach,
  isAdmin,
  onRemoveCoach,
  isRemovingCoach,
  last = false,
}: GroupLeadRowsProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const agentName = group.agent_title ?? t('app.aiAgent');

  return (
    <>
      {/* The agent's own thread hashes on its id, so the agent takes the same
          colour here as in the conversation list. */}
      <RosterRow
        avatar={
          <InitialsAvatar
            initials={initialsFor(agentName)}
            slot={avatarSlot({ id: group.agent_id, agent_id: group.agent_id, group_id: null })}
          />
        }
        name={agentName}
        nameSuffix={group.agent_handle ? `· ${MENTION_PREFIX}${group.agent_handle}` : null}
        subtitle={t('app.aiAgent')}
        testID="group-info-ai-coach"
      />

      {group.coach_user_id ? (
        <CoachRow
          coachUserId={group.coach_user_id}
          name={group.coach_display_name ?? t('app.unknownMember')}
          viewerIsCoach={viewerIsCoach}
          canRemove={isAdmin}
          onRemove={onRemoveCoach}
          isRemoving={isRemovingCoach}
          last={last}
        />
      ) : (
        <RosterRow
          // A muted glyph in the avatar column, so the line starts where the
          // names do; nobody is there to draw an avatar for.
          avatar={
            <View className="w-10 h-10 items-center justify-center">
              <Feather name="user" size={18} color={colors.text.tertiary} />
            </View>
          }
          // An admin can bring a coach in from the Invites section; a member
          // cannot, so they are told only that there is none.
          name={isAdmin ? t('humanCoach.none') : t('humanCoach.noneMember')}
          last={last}
          testID="group-info-no-coach"
        />
      )}
    </>
  );
}

interface CoachRowProps {
  coachUserId: string;
  name: string;
  viewerIsCoach: boolean;
  canRemove: boolean;
  onRemove: () => void;
  isRemoving: boolean;
  last: boolean;
}

/** The attached human coach — a person, so the same avatar hash a member row takes. */
function CoachRow({ coachUserId, name, viewerIsCoach, canRemove, onRemove, isRemoving, last }: CoachRowProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();

  return (
    <RosterRow
      avatar={
        <InitialsAvatar
          initials={initialsFor(name)}
          slot={avatarSlot({ id: coachUserId, agent_id: null, group_id: null })}
        />
      }
      name={name}
      nameSuffix={viewerIsCoach ? t('groups.youSuffix') : null}
      subtitle={t('humanCoach.coach')}
      last={last}
      testID="group-info-human-coach"
      trailing={
        canRemove ? (
          <TouchableOpacity
            className="p-2"
            onPress={onRemove}
            disabled={isRemoving}
            accessibilityRole="button"
            accessibilityLabel={t('humanCoach.remove')}
            testID="remove-coach-button"
          >
            {isRemoving ? (
              <ActivityIndicator size="small" color={colors.text.secondary} />
            ) : (
              <Feather name="user-minus" size={18} color={colors.text.secondary} />
            )}
          </TouchableOpacity>
        ) : undefined
      }
    />
  );
}
