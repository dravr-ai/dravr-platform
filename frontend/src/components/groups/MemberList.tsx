// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Sortable member list table for a coaching group
// ABOUTME: Displays name, role, join date, peer sharing status, and admin actions

import { useState } from 'react';
import { clsx } from 'clsx';
import { Shield, Crown, User, ChevronUp, ChevronDown, Trash2 } from 'lucide-react';
import { Button, ConfirmDialog, useErrorToast, useSuccessToast } from '../ui';
import { useRemoveMember, useUpdateMemberRole } from '../../hooks/useGroups';
import type { GroupMember, GroupRole } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';

interface MemberListProps {
  groupId: string;
  members: GroupMember[];
  currentUserRole: GroupRole;
  currentUserId: string;
  isLoading: boolean;
}

type SortField = 'display_name' | 'role' | 'joined_at' | 'peer_sharing_consent';
type SortDirection = 'asc' | 'desc';

const ROLE_ORDER: Record<GroupRole, number> = { owner: 0, admin: 1, member: 2 };

const ROLE_BADGE: Record<GroupRole, { labelKey: string; color: string; Icon: typeof Crown }> = {
  owner: { labelKey: 'groups.owner', color: 'bg-warning/20 text-on-warning-container', Icon: Crown },
  admin: { labelKey: 'groups.admin', color: 'bg-primary/20 text-primary', Icon: Shield },
  member: { labelKey: 'groups.member', color: 'bg-surface-container-high/20 text-on-surface-variant', Icon: User },
};

function formatDate(dateStr: string): string {
  return new Date(dateStr).toLocaleDateString(undefined, {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
  });
}

export default function MemberList({
  groupId,
  members,
  currentUserRole,
  currentUserId,
  isLoading,
}: MemberListProps) {
  const { t } = useTranslation();
  const [sortField, setSortField] = useState<SortField>('role');
  const [sortDirection, setSortDirection] = useState<SortDirection>('asc');
  const [confirmRemove, setConfirmRemove] = useState<GroupMember | null>(null);
  const { removeMember, isPending: isRemoving } = useRemoveMember(groupId);
  const { updateRole, isPending: isUpdatingRole } = useUpdateMemberRole(groupId);
  const showError = useErrorToast();
  const showSuccess = useSuccessToast();

  const canManageMembers = currentUserRole === 'owner' || currentUserRole === 'admin';
  const isOwner = currentUserRole === 'owner';

  const handleSort = (field: SortField) => {
    if (sortField === field) {
      setSortDirection((prev) => (prev === 'asc' ? 'desc' : 'asc'));
    } else {
      setSortField(field);
      setSortDirection('asc');
    }
  };

  const sortedMembers = [...members].sort((a, b) => {
    const direction = sortDirection === 'asc' ? 1 : -1;
    switch (sortField) {
      case 'display_name': {
        const nameA = (a.display_name ?? '').toLowerCase();
        const nameB = (b.display_name ?? '').toLowerCase();
        return nameA.localeCompare(nameB) * direction;
      }
      case 'role':
        return (ROLE_ORDER[a.role] - ROLE_ORDER[b.role]) * direction;
      case 'joined_at':
        return (new Date(a.joined_at).getTime() - new Date(b.joined_at).getTime()) * direction;
      case 'peer_sharing_consent':
        return ((a.peer_sharing_consent ? 1 : 0) - (b.peer_sharing_consent ? 1 : 0)) * direction;
      default:
        return 0;
    }
  });

  const handleRemove = async () => {
    if (!confirmRemove) return;
    try {
      await removeMember(confirmRemove.user_id);
      showSuccess(t('app.memberRemoved'), t('app.memberRemovedFrom', { member: confirmRemove.display_name ?? t('groups.member') }));
      setConfirmRemove(null);
    } catch (err) {
      const message = err instanceof Error ? err.message : t('groups.removeFailed');
      showError(t('app.removeFailed'), message);
    }
  };

  const handlePromote = async (member: GroupMember) => {
    const newRole: GroupRole = member.role === 'member' ? 'admin' : 'member';
    try {
      await updateRole({ userId: member.user_id, role: newRole });
      showSuccess(t('app.roleUpdated'), t('app.memberIsNowRole', { member: member.display_name ?? t('groups.member'), role: newRole }));
    } catch (err) {
      const message = err instanceof Error ? err.message : t('groups.roleUpdateFailed');
      showError(t('app.updateFailed'), message);
    }
  };

  const SortIcon = ({ field }: { field: SortField }) => {
    if (sortField !== field) return null;
    return sortDirection === 'asc' ? (
      <ChevronUp className="w-3.5 h-3.5 inline ml-1" />
    ) : (
      <ChevronDown className="w-3.5 h-3.5 inline ml-1" />
    );
  };

  if (isLoading) {
    return (
      <div className="flex justify-center py-8">
        <div className="pierre-spinner" />
      </div>
    );
  }

  if (members.length === 0) {
    return (
      <p className="text-center py-8 text-outline">{t('groups.noMembers')}</p>
    );
  }

  return (
    <>
      <div className="overflow-x-auto">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b ghost-border text-left">
              <th
                className="py-3 px-4 text-on-surface-variant font-medium cursor-pointer select-none hover:text-on-surface transition-colors"
                onClick={() => handleSort('display_name')}
              >
                {t('groups.colName')} <SortIcon field="display_name" />
              </th>
              <th
                className="py-3 px-4 text-on-surface-variant font-medium cursor-pointer select-none hover:text-on-surface transition-colors"
                onClick={() => handleSort('role')}
              >
                {t('settingsUi.role')} <SortIcon field="role" />
              </th>
              <th
                className="py-3 px-4 text-on-surface-variant font-medium cursor-pointer select-none hover:text-on-surface transition-colors"
                onClick={() => handleSort('joined_at')}
              >
                {t('groups.colJoined')} <SortIcon field="joined_at" />
              </th>
              <th
                className="py-3 px-4 text-on-surface-variant font-medium cursor-pointer select-none hover:text-on-surface transition-colors"
                onClick={() => handleSort('peer_sharing_consent')}
              >
                {t('groups.peerSharing')} <SortIcon field="peer_sharing_consent" />
              </th>
              {canManageMembers && (
                <th className="py-3 px-4 text-on-surface-variant font-medium text-right">
                  {t('groups.colActions')}
                </th>
              )}
            </tr>
          </thead>
          <tbody>
            {sortedMembers.map((member) => {
              const badge = ROLE_BADGE[member.role];
              const BadgeIcon = badge.Icon;
              const isSelf = member.user_id === currentUserId;
              const canRemove = canManageMembers && !isSelf && member.role !== 'owner';
              const canChangeRole = isOwner && !isSelf && member.role !== 'owner';

              return (
                <tr
                  key={member.id}
                  className={clsx(
                    'border-b ghost-border transition-colors',
                    isSelf ? 'bg-primary/5' : 'hover:bg-surface-container-low'
                  )}
                >
                  <td className="py-3 px-4">
                    <span className="text-on-surface font-medium">
                      {member.display_name ?? t('settingsUi.unknownDate')}
                      {isSelf && (
                        <span className="ml-2 text-xs text-outline">(you)</span>
                      )}
                    </span>
                  </td>
                  <td className="py-3 px-4">
                    <span
                      className={clsx(
                        'inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-medium',
                        badge.color
                      )}
                    >
                      <BadgeIcon className="w-3 h-3" />
                      {t(badge.labelKey)}
                    </span>
                  </td>
                  <td className="py-3 px-4 text-on-surface-variant">
                    {formatDate(member.joined_at)}
                  </td>
                  <td className="py-3 px-4">
                    {member.peer_sharing_consent ? (
                      <span className="text-activity text-xs font-medium">{t('groups.enabled')}</span>
                    ) : (
                      <span className="text-outline text-xs">{t('groups.disabled')}</span>
                    )}
                  </td>
                  {canManageMembers && (
                    <td className="py-3 px-4 text-right">
                      <div className="flex items-center justify-end gap-2">
                        {canChangeRole && (
                          <Button
                            variant="secondary"
                            size="sm"
                            onClick={() => handlePromote(member)}
                            loading={isUpdatingRole}
                            title={member.role === 'member' ? t('groups.promoteToAdmin') : t('groups.demoteToMember')}
                            aria-label={member.role === 'member' ? t('groups.promoteNamed', { name: member.display_name ?? t('groups.memberFallback') }) : t('groups.demoteNamed', { name: member.display_name ?? t('groups.memberFallback') })}
                          >
                            {member.role === 'member' ? (
                              <Shield className="w-4 h-4" />
                            ) : (
                              <User className="w-4 h-4" />
                            )}
                          </Button>
                        )}
                        {canRemove && (
                          <Button
                            variant="danger"
                            size="sm"
                            onClick={() => setConfirmRemove(member)}
                            title={t('groups.removeMemberAria')}
                            aria-label={t('groups.removeNamed', { name: member.display_name ?? t('groups.memberFallback') })}
                          >
                            <Trash2 className="w-4 h-4" />
                          </Button>
                        )}
                      </div>
                    </td>
                  )}
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>

      {/* Confirm removal dialog */}
      <ConfirmDialog
        isOpen={!!confirmRemove}
        onClose={() => setConfirmRemove(null)}
        onConfirm={handleRemove}
        title={t('groups.removeMember')}
        message={t('app.confirmRemoveMemberWeb', { member: confirmRemove?.display_name ?? t('app.thisMember') })}
        confirmLabel={t('app.remove')}
        variant="danger"
        isLoading={isRemoving}
      />
    </>
  );
}
