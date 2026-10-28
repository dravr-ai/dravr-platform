// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Sortable member list table for a coaching group, sized for the Group info side panel
// ABOUTME: Displays name with join date and peer sharing under it, role, admin actions, and the coach's way into each athlete's plan

import { useState } from 'react';
import { clsx } from 'clsx';
import { Shield, User, ChevronUp, ChevronDown, Trash2, Clipboard, ArrowUp, ArrowDown } from 'lucide-react';
import { Button, ConfirmDialog, Select, useErrorToast, useSuccessToast } from '../ui';
import RoleBadge from './RoleBadge';
import { useRemoveMember, useUpdateMemberRole } from '../../hooks/useGroups';
import type { GroupMember, GroupRole } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import { formatDate, PLAN_THREAD_KEYS } from '@pierre/chat-utils';
import { describeApiError } from '@pierre/ui-logic';

interface MemberListProps {
  groupId: string;
  members: GroupMember[];
  currentUserRole: GroupRole;
  currentUserId: string;
  isLoading: boolean;
  /**
   * Open the caller's plan thread for a member. Passed only for the group's
   * coach, so the action never reaches a member's view of the roster. A coach
   * who is also a member never gets it on their own row: the server refuses
   * a coach acting for themselves.
   */
  onWorkOnPlan?: (member: GroupMember) => void;
  /** The member whose plan thread is being opened, while one is. */
  openingPlanFor?: string | null;
}

/**
 * What the list sorts by. The table lives in the Group info panel (448px at
 * most), which has room for three columns with the Actions column whole
 * (carnet#1191): the join date and the sharing state read as one line under
 * the name. The name
 * and role headers still sort on a click; every field, the join date and the
 * sharing state included, is picked from the compact control above the table,
 * which costs height rather than width.
 */
type SortField = 'display_name' | 'role' | 'joined_at' | 'peer_sharing_consent';
type SortDirection = 'asc' | 'desc';

/** The sort control's fields, in menu order, with their catalogue labels. */
const SORT_FIELD_LABEL_KEY: Record<SortField, string> = {
  display_name: 'groups.colName',
  role: 'settingsUi.role',
  joined_at: 'groups.colJoined',
  peer_sharing_consent: 'groups.peerSharing',
};

function isSortField(value: string): value is SortField {
  return Object.prototype.hasOwnProperty.call(SORT_FIELD_LABEL_KEY, value);
}

const ROLE_ORDER: Record<GroupRole, number> = { owner: 0, admin: 1, member: 2 };

export default function MemberList({
  groupId,
  members,
  currentUserRole,
  currentUserId,
  isLoading,
  onWorkOnPlan,
  openingPlanFor = null,
}: MemberListProps) {
  const { t, i18n } = useTranslation();
  const [sortField, setSortField] = useState<SortField>('role');
  const [sortDirection, setSortDirection] = useState<SortDirection>('asc');
  const [confirmRemove, setConfirmRemove] = useState<GroupMember | null>(null);
  const { removeMember, isPending: isRemoving } = useRemoveMember(groupId);
  const { updateRole, isPending: isUpdatingRole } = useUpdateMemberRole(groupId);
  const showError = useErrorToast();
  const showSuccess = useSuccessToast();

  const canManageMembers = currentUserRole === 'owner' || currentUserRole === 'admin';
  const isOwner = currentUserRole === 'owner';
  const hasActions = canManageMembers || onWorkOnPlan !== undefined;

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
    }
  });

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

  const directionLabel = sortDirection === 'asc' ? t('groups.sortAscending') : t('groups.sortDescending');

  return (
    <>
      {/* The sort control: one line above the table, so the four fields it
          offers never widen the panel. It shares its state with the headers. */}
      <div className="flex items-center justify-end gap-2 px-3 pb-2" data-testid="member-sort">
        <span className="text-xs text-on-surface-variant whitespace-nowrap">{t('groups.sortBy')}</span>
        <div className="w-36">
          <Select
            size="sm"
            aria-label={t('groups.sortBy')}
            value={sortField}
            onChange={(event) => {
              if (isSortField(event.target.value)) {
                setSortField(event.target.value);
                setSortDirection('asc');
              }
            }}
            options={(Object.keys(SORT_FIELD_LABEL_KEY) as SortField[]).map((field) => ({
              value: field,
              label: t(SORT_FIELD_LABEL_KEY[field]),
            }))}
            data-testid="member-sort-field"
          />
        </div>
        <Button
          variant="tertiary"
          size="sm"
          onClick={() => setSortDirection((prev) => (prev === 'asc' ? 'desc' : 'asc'))}
          title={directionLabel}
          aria-label={directionLabel}
          data-testid="member-sort-direction"
        >
          {sortDirection === 'asc' ? (
            <ArrowUp className="w-4 h-4" aria-hidden="true" />
          ) : (
            <ArrowDown className="w-4 h-4" aria-hidden="true" />
          )}
        </Button>
      </div>
      <div className="overflow-x-auto" data-testid="member-list">
        <table className="w-full table-fixed text-sm">
          <thead>
            <tr className="border-b ghost-border text-left">
              <th
                className="py-3 px-3 text-on-surface-variant font-medium cursor-pointer select-none hover:text-on-surface transition-colors"
                onClick={() => handleSort('display_name')}
              >
                {t('groups.colName')} <SortIcon field="display_name" />
              </th>
              <th
                className="w-28 py-3 px-3 text-on-surface-variant font-medium cursor-pointer select-none hover:text-on-surface transition-colors"
                onClick={() => handleSort('role')}
              >
                {t('settingsUi.role')} <SortIcon field="role" />
              </th>
              {hasActions && (
                <th className="w-36 py-3 px-3 text-on-surface-variant font-medium text-right">
                  {t('groups.colActions')}
                </th>
              )}
            </tr>
          </thead>
          <tbody>
            {sortedMembers.map((member) => {
              const isSelf = member.user_id === currentUserId;
              const canRemove = canManageMembers && !isSelf && member.role !== 'owner';
              const canChangeRole = isOwner && !isSelf && member.role !== 'owner';
              const memberName = member.display_name ?? t('groups.memberFallback');
              const workOnPlanLabel = t(PLAN_THREAD_KEYS.open, { name: memberName });

              return (
                <tr
                  key={member.id}
                  className={clsx(
                    'border-b ghost-border transition-colors',
                    isSelf ? 'bg-primary/5' : 'hover:bg-surface-container-low'
                  )}
                >
                  <td className="min-w-0 py-2.5 px-3">
                    <span className="block truncate text-on-surface font-medium" title={member.display_name ?? undefined}>
                      {member.display_name ?? t('settingsUi.unknownDate')}
                      {isSelf && (
                        <span className="ml-2 text-xs text-outline">{t('groups.youSuffix')}</span>
                      )}
                    </span>
                    {/* The join date and the sharing state, wrapping onto a
                        second line rather than widening the panel. */}
                    <span
                      className="flex flex-wrap gap-x-1.5 text-xs text-on-surface-variant"
                      data-testid={`member-detail-${member.user_id}`}
                    >
                      <span className="whitespace-nowrap">
                        {t('groups.colJoined')}{' '}
                        {formatDate(member.joined_at, i18n.language)}
                      </span>
                      {member.peer_sharing_consent ? (
                        <span className="whitespace-nowrap text-activity font-medium">{t('groups.sharingOn')}</span>
                      ) : (
                        <span className="whitespace-nowrap text-outline">{t('groups.sharingOff')}</span>
                      )}
                    </span>
                  </td>
                  <td className="py-2.5 px-3">
                    <RoleBadge kind={member.role} />
                  </td>
                  {hasActions && (
                    <td className="py-2.5 px-3 text-right">
                      <div className="flex items-center justify-end gap-1">
                        {onWorkOnPlan && !isSelf && (
                          <Button
                            variant="tertiary"
                            size="sm"
                            onClick={() => onWorkOnPlan(member)}
                            loading={openingPlanFor === member.user_id}
                            disabled={openingPlanFor !== null}
                            title={workOnPlanLabel}
                            aria-label={workOnPlanLabel}
                            data-testid={`member-work-on-plan-${member.user_id}`}
                          >
                            <Clipboard className="w-4 h-4" aria-hidden="true" />
                          </Button>
                        )}
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
