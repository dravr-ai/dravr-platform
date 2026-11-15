// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: What an owner or admin may do to another member from their `⋯` — change their role, remove them — and the confirm
// ABOUTME: One source for the member rows and the coach's athlete cards, so both offer the same actions on the same rules

import { useState, type ReactNode } from 'react';
import type { GroupMember, GroupRole } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import { describeApiError } from '@pierre/ui-logic';
import { ConfirmDialog, useErrorToast, useSuccessToast } from '../ui';
import type { OverflowMenuItem } from '../ui';
import { useRemoveMember, useUpdateMemberRole } from '../../hooks/useGroups';

export interface MemberActions {
  /** The `⋯` items the viewer may use on `member`: none on their own row or the owner's. */
  itemsFor: (member: GroupMember) => OverflowMenuItem[];
  /** The removal confirm, mounted once by whoever lists the members. */
  dialog: ReactNode;
}

/**
 * The owner changes a member's role; an owner or admin removes a member.
 * Nobody acts on themselves or on the owner, as the server refuses both.
 * Removal is drawn in the confirm alone: the row's menu only names it.
 */
export function useMemberActions(groupId: string, currentUserRole: GroupRole, currentUserId: string): MemberActions {
  const { t } = useTranslation();
  const [confirmRemove, setConfirmRemove] = useState<GroupMember | null>(null);
  const { removeMember, isPending: isRemoving } = useRemoveMember(groupId);
  const { updateRole } = useUpdateMemberRole(groupId);
  const showError = useErrorToast();
  const showSuccess = useSuccessToast();

  const canManageMembers = currentUserRole === 'owner' || currentUserRole === 'admin';
  const isOwner = currentUserRole === 'owner';
  const nameOf = (member: GroupMember) => member.display_name ?? t('groups.memberFallback');

  const handleRemove = async () => {
    if (!confirmRemove) return;
    try {
      await removeMember(confirmRemove.user_id);
      showSuccess(t('app.memberRemoved'), t('app.memberRemovedFrom', { member: confirmRemove.display_name ?? t('groups.member') }));
      setConfirmRemove(null);
    } catch (err) {
      const message = describeApiError(err, { t, fallbackKey: 'groups.removeFailed' });
      showError(t('app.removeFailed'), message);
    }
  };

  const handlePromote = async (member: GroupMember) => {
    const newRole: GroupRole = member.role === 'member' ? 'admin' : 'member';
    try {
      await updateRole({ userId: member.user_id, role: newRole });
      showSuccess(t('app.roleUpdated'), t('app.memberIsNowRole', { member: member.display_name ?? t('groups.member'), role: newRole }));
    } catch (err) {
      const message = describeApiError(err, { t, fallbackKey: 'groups.roleUpdateFailed' });
      showError(t('app.updateFailed'), message);
    }
  };

  const itemsFor = (member: GroupMember): OverflowMenuItem[] => {
    if (member.user_id === currentUserId || member.role === 'owner') return [];
    const items: OverflowMenuItem[] = [];
    if (isOwner) {
      items.push({
        label: member.role === 'member' ? t('groups.promoteToAdmin') : t('groups.demoteToMember'),
        onSelect: () => void handlePromote(member),
        'data-testid': `member-role-${member.user_id}`,
      });
    }
    if (canManageMembers) {
      items.push({
        label: t('groups.info.removeFromGroup'),
        danger: true,
        onSelect: () => setConfirmRemove(member),
        'data-testid': `member-remove-${member.user_id}`,
      });
    }
    return items;
  };

  const dialog = (
    <ConfirmDialog
      isOpen={!!confirmRemove}
      onClose={() => setConfirmRemove(null)}
      onConfirm={() => void handleRemove()}
      title={t('groups.info.removeTitle', { name: confirmRemove ? nameOf(confirmRemove) : '' })}
      message={t('app.confirmRemoveMemberWeb', { member: confirmRemove?.display_name ?? t('app.thisMember') })}
      confirmLabel={t('app.remove')}
      variant="danger"
      isLoading={isRemoving}
    />
  );

  return { itemsFor, dialog };
}
