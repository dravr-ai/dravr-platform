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

const ROLE_BADGE: Record<GroupRole, { label: string; color: string; Icon: typeof Crown }> = {
  owner: { label: 'Owner', color: 'bg-amber-500/20 text-amber-400', Icon: Crown },
  admin: { label: 'Admin', color: 'bg-pierre-violet/20 text-pierre-violet-light', Icon: Shield },
  member: { label: 'Member', color: 'bg-zinc-500/20 text-zinc-400', Icon: User },
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
      showSuccess('Member removed', `${confirmRemove.display_name ?? 'Member'} has been removed from the group.`);
      setConfirmRemove(null);
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to remove member';
      showError('Remove failed', message);
    }
  };

  const handlePromote = async (member: GroupMember) => {
    const newRole: GroupRole = member.role === 'member' ? 'admin' : 'member';
    try {
      await updateRole({ userId: member.user_id, role: newRole });
      showSuccess('Role updated', `${member.display_name ?? 'Member'} is now ${newRole}.`);
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to update role';
      showError('Update failed', message);
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
      <p className="text-center py-8 text-zinc-500">No members in this group.</p>
    );
  }

  return (
    <>
      <div className="overflow-x-auto">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-white/10 text-left">
              <th
                className="py-3 px-4 text-zinc-400 font-medium cursor-pointer select-none hover:text-white transition-colors"
                onClick={() => handleSort('display_name')}
              >
                Name <SortIcon field="display_name" />
              </th>
              <th
                className="py-3 px-4 text-zinc-400 font-medium cursor-pointer select-none hover:text-white transition-colors"
                onClick={() => handleSort('role')}
              >
                Role <SortIcon field="role" />
              </th>
              <th
                className="py-3 px-4 text-zinc-400 font-medium cursor-pointer select-none hover:text-white transition-colors"
                onClick={() => handleSort('joined_at')}
              >
                Joined <SortIcon field="joined_at" />
              </th>
              <th
                className="py-3 px-4 text-zinc-400 font-medium cursor-pointer select-none hover:text-white transition-colors"
                onClick={() => handleSort('peer_sharing_consent')}
              >
                Peer Sharing <SortIcon field="peer_sharing_consent" />
              </th>
              {canManageMembers && (
                <th className="py-3 px-4 text-zinc-400 font-medium text-right">
                  Actions
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
                    'border-b border-white/5 transition-colors',
                    isSelf ? 'bg-pierre-violet/5' : 'hover:bg-white/5'
                  )}
                >
                  <td className="py-3 px-4">
                    <span className="text-white font-medium">
                      {member.display_name ?? 'Unknown'}
                      {isSelf && (
                        <span className="ml-2 text-xs text-zinc-500">(you)</span>
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
                      {badge.label}
                    </span>
                  </td>
                  <td className="py-3 px-4 text-zinc-400">
                    {formatDate(member.joined_at)}
                  </td>
                  <td className="py-3 px-4">
                    {member.peer_sharing_consent ? (
                      <span className="text-pierre-activity text-xs font-medium">Enabled</span>
                    ) : (
                      <span className="text-zinc-500 text-xs">Disabled</span>
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
                            title={member.role === 'member' ? 'Promote to Admin' : 'Demote to Member'}
                            aria-label={member.role === 'member' ? `Promote ${member.display_name ?? 'member'} to admin` : `Demote ${member.display_name ?? 'member'} to member`}
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
                            title="Remove member"
                            aria-label={`Remove ${member.display_name ?? 'member'} from group`}
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
        title="Remove Member"
        message={`Are you sure you want to remove ${confirmRemove?.display_name ?? 'this member'} from the group? They can rejoin with a new invite.`}
        confirmLabel="Remove"
        variant="danger"
        isLoading={isRemoving}
      />
    </>
  );
}
