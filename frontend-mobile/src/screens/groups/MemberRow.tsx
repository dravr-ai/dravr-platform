// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: One member of a coaching group — avatar, display name, role and consent as a 52-tall row, admin controls trailing
// ABOUTME: Drawn by Group info inside the group's own chat thread; the roles it offers match what the API allows

import React, { useMemo } from 'react';
import { Text, TouchableOpacity, ActivityIndicator } from 'react-native';
import { Feather } from '@expo/vector-icons';
import { avatarSlot, initialsFor } from '@pierre/chat-utils';
import { useThemeColors } from '../../constants/theme';
import { InitialsAvatar } from '../../components/ui/InitialsAvatar';
import { RosterRow } from './RosterRow';
import type { GroupMember, GroupRole } from '../../types';
import { useTranslation } from '@pierre/i18n';

/**
 * The corpus key for each role badge. Module scope, so it holds keys and the
 * row resolves one — the same shape every other label table in the app uses.
 */
export const ROLE_LABEL_KEYS: Record<GroupRole, string> = {
  owner: 'app.roleOwner',
  admin: 'app.roleAdmin',
  member: 'app.roleMember',
};

/** What a member row says when the server has no display name for the row. */
const UNKNOWN_MEMBER_KEY = 'app.unknownMember';

export interface MemberRowProps {
  member: GroupMember;
  /** The caller owns or administers the group: may remove plain members. */
  isAdmin: boolean;
  /** The caller owns the group: may promote and demote. */
  isOwner: boolean;
  onRemove: (member: GroupMember) => void;
  onChangeRole: (member: GroupMember, role: GroupRole) => void;
  isRemoving: boolean;
  isChangingRole: boolean;
  /** The last row of the members list draws no hairline under itself. */
  last?: boolean;
}

/**
 * A member, as a `RosterRow`: the display name over a role line (role, then
 * consent if the member has granted it), and admin controls trailing. The
 * group's AI agent and human coach draw through the same row above the
 * members, so the three kinds of people read as one list.
 */
export function MemberRow({
  member,
  isAdmin,
  isOwner,
  onRemove,
  onChangeRole,
  isRemoving,
  isChangingRole,
  last = false,
}: MemberRowProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const displayName = member.display_name ?? t(UNKNOWN_MEMBER_KEY);
  // The same initials and the same colour hash the conversation list uses, so
  // one person looks like one person wherever the app draws them.
  const slot = avatarSlot({ id: member.user_id, agent_id: null, group_id: null });
  const subtitle = useMemo(() => {
    const parts = [t(ROLE_LABEL_KEYS[member.role])];
    if (member.peer_sharing_consent) {
      parts.push(t('app.sharing'));
    }
    return parts.join(' · ');
  }, [member.role, member.peer_sharing_consent, t]);

  return (
    <RosterRow
      avatar={<InitialsAvatar initials={initialsFor(displayName)} slot={slot} />}
      name={displayName}
      subtitle={subtitle}
      last={last}
      testID={`group-member-${member.user_id}`}
      trailing={
        <>
          {/* Promotion is the owner's call — the API rejects it from anyone else,
              so showing it to a plain admin would advertise a 403. */}
          {isOwner && member.role !== 'owner' && (
            <TouchableOpacity
              className="px-2 py-1 ml-1.5 rounded border border-border-strong"
              onPress={() => onChangeRole(member, member.role === 'admin' ? 'member' : 'admin')}
              disabled={isChangingRole}
              testID={`member-role-${member.user_id}`}
            >
              {isChangingRole ? (
                <ActivityIndicator size="small" color={colors.text.secondary} />
              ) : (
                <Text className="text-xs font-semibold text-text-secondary">
                  {member.role === 'admin' ? t('app.demote') : t('app.promote')}
                </Text>
              )}
            </TouchableOpacity>
          )}
          {isAdmin && member.role === 'member' && (
            <TouchableOpacity
              className="p-2"
              onPress={() => onRemove(member)}
              disabled={isRemoving}
              testID={`member-remove-${member.user_id}`}
            >
              {isRemoving ? (
                <ActivityIndicator size="small" color={colors.text.secondary} />
              ) : (
                <Feather name="user-minus" size={18} color={colors.text.secondary} />
              )}
            </TouchableOpacity>
          )}
        </>
      }
    />
  );
}
