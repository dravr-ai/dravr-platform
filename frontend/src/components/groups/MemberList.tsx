// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A coaching group's members as Group info rows — avatar, name, role word, sharing state, and a `⋯` menu for role and removal
// ABOUTME: A sort control appears only once the group is big enough to need one; removal always asks first, in the dialog

import { useState } from 'react';
import { ArrowDown, ArrowUp } from 'lucide-react';
import { IconButton, OverflowMenu, Select } from '../ui';
import RoleWord from './RoleWord';
import { PersonAvatar, PersonRow, ROW_DIVIDER, SharingState } from './GroupInfoParts';
import { useMemberActions } from './useMemberActions';
import type { GroupMember, GroupRole } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';

interface MemberListProps {
  groupId: string;
  members: GroupMember[];
  currentUserRole: GroupRole;
  currentUserId: string;
  isLoading: boolean;
  /**
   * Leave the viewer's own row out, where their slot is drawn elsewhere: the
   * owner offered the coach seat reads themselves in the coach slot.
   */
  hideSelf?: boolean;
}

/**
 * What the list sorts by, once there is a control to pick it. Every field
 * the roster carries can order it (carnet#1191), the join date and the
 * sharing state included.
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

/**
 * Up to this many people the list is read at a glance and a sort control is
 * chrome (a pilot group is one coach and one athlete); above it the control
 * earns its line.
 */
const SORT_CONTROL_ABOVE = 8;

function isSortField(value: string): value is SortField {
  return Object.prototype.hasOwnProperty.call(SORT_FIELD_LABEL_KEY, value);
}

const ROLE_ORDER: Record<GroupRole, number> = { owner: 0, admin: 1, member: 2 };

function compareMembers(a: GroupMember, b: GroupMember, field: SortField): number {
  switch (field) {
    case 'display_name':
      return (a.display_name ?? '').toLowerCase().localeCompare((b.display_name ?? '').toLowerCase());
    case 'role':
      return ROLE_ORDER[a.role] - ROLE_ORDER[b.role];
    case 'joined_at':
      return new Date(a.joined_at).getTime() - new Date(b.joined_at).getTime();
    case 'peer_sharing_consent':
      return (a.peer_sharing_consent ? 1 : 0) - (b.peer_sharing_consent ? 1 : 0);
  }
}

export default function MemberList({
  groupId,
  members,
  currentUserRole,
  currentUserId,
  isLoading,
  hideSelf = false,
}: MemberListProps) {
  const { t } = useTranslation();
  const [sortField, setSortField] = useState<SortField>('role');
  const [sortDirection, setSortDirection] = useState<SortDirection>('asc');
  const actions = useMemberActions(groupId, currentUserRole, currentUserId);

  const listed = hideSelf ? members.filter((m) => m.user_id !== currentUserId) : members;
  const direction = sortDirection === 'asc' ? 1 : -1;
  const sortedMembers = [...listed].sort((a, b) => compareMembers(a, b, sortField) * direction);
  const showsSort = SORT_CONTROL_ABOVE < listed.length;

  if (isLoading) {
    return (
      <div className={`flex justify-center py-6 ${ROW_DIVIDER}`}>
        <div className="pierre-spinner" />
      </div>
    );
  }

  if (sortedMembers.length === 0) return null;

  const directionLabel = sortDirection === 'asc' ? t('groups.sortAscending') : t('groups.sortDescending');

  return (
    <div className={ROW_DIVIDER} data-testid="member-list">
      {showsSort && (
        <div className="flex items-center justify-end gap-2 px-4 py-2" data-testid="member-sort">
          <span className="whitespace-nowrap text-xs text-on-surface-variant">{t('groups.sortBy')}</span>
          <div className="w-44">
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
          <IconButton
            onClick={() => setSortDirection((prev) => (prev === 'asc' ? 'desc' : 'asc'))}
            title={directionLabel}
            aria-label={directionLabel}
            data-testid="member-sort-direction"
          >
            {sortDirection === 'asc' ? (
              <ArrowUp className="h-4 w-4" aria-hidden="true" />
            ) : (
              <ArrowDown className="h-4 w-4" aria-hidden="true" />
            )}
          </IconButton>
        </div>
      )}
      <ul aria-label={t('groups.tabMembers')}>
        {sortedMembers.map((member) => {
          const isSelf = member.user_id === currentUserId;
          const name = member.display_name ?? t('groups.memberFallback');
          const items = actions.itemsFor(member);
          return (
            <li key={member.id} className={ROW_DIVIDER} data-testid={`member-row-${member.user_id}`}>
              <PersonRow
                className={isSelf ? 'bg-primary/5' : undefined}
                avatar={<PersonAvatar userId={member.user_id} name={name} />}
                title={
                  <>
                    <span title={member.display_name ?? undefined}>{name}</span>
                    {isSelf && <span className="ml-2 text-xs font-normal text-outline">{t('groups.youSuffix')}</span>}
                  </>
                }
                detail={
                  <span className="inline-flex flex-wrap items-center gap-x-1.5" data-testid={`member-detail-${member.user_id}`}>
                    <RoleWord kind={member.role} />
                    <span aria-hidden="true">·</span>
                    <SharingState on={member.peer_sharing_consent} onLabel={t('groups.sharingOn')} offLabel={t('groups.sharingOff')} />
                  </span>
                }
                trailing={
                  items.length > 0 ? (
                    <OverflowMenu
                      label={t('groups.info.moreActions', { name })}
                      items={items}
                      data-testid={`member-menu-${member.user_id}`}
                    />
                  ) : undefined
                }
              />
            </li>
          );
        })}
      </ul>
      {actions.dialog}
    </div>
  );
}
