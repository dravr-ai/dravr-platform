// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the group member list's sort: the name and role headers, and the compact control above the table
// ABOUTME: Red if the join date or the sharing state can no longer order the list, or the control and headers disagree (carnet#1191)

import { describe, it, expect, vi } from 'vitest';
import { fireEvent, render, screen, within } from '@testing-library/react';
import type { GroupMember } from '@pierre/shared-types';
import { i18n } from '@pierre/i18n';
import MemberList from '../MemberList';
import { ToastProvider } from '../../ui';

vi.mock('../../../hooks/useGroups', () => ({
  useRemoveMember: () => ({ removeMember: vi.fn(), isPending: false }),
  useUpdateMemberRole: () => ({ updateRole: vi.fn(), isPending: false }),
}));

function member(overrides: Partial<GroupMember> & Pick<GroupMember, 'user_id' | 'display_name'>): GroupMember {
  return {
    id: `m-${overrides.user_id}`,
    group_id: 'group-1',
    role: 'member',
    peer_sharing_consent: false,
    consent_given_at: '2026-01-01T00:00:00Z',
    joined_at: '2026-01-01T00:00:00Z',
    ...overrides,
  };
}

// Joined in the order Cleo, Alice, Ben; only Ben shares.
const MEMBERS: GroupMember[] = [
  member({ user_id: 'alice', display_name: 'Alice', role: 'owner', joined_at: '2026-03-01T00:00:00Z' }),
  member({ user_id: 'ben', display_name: 'Ben', joined_at: '2026-05-01T00:00:00Z', peer_sharing_consent: true }),
  member({ user_id: 'cleo', display_name: 'Cleo', role: 'admin', joined_at: '2026-01-15T00:00:00Z' }),
];

function renderList() {
  return render(
    <ToastProvider>
      <MemberList
        groupId="group-1"
        members={MEMBERS}
        currentUserRole="owner"
        currentUserId="alice"
        isLoading={false}
      />
    </ToastProvider>,
  );
}

/** The member names, top to bottom. */
function order(): string[] {
  const rows = within(screen.getByTestId('member-list')).getAllByRole('row').slice(1);
  return rows.map((row) => within(row).getAllByRole('cell')[0]?.querySelector('span')?.firstChild?.textContent ?? '');
}

describe('MemberList sort', () => {
  it('orders by role until asked otherwise', () => {
    renderList();
    expect(order()).toEqual(['Alice', 'Cleo', 'Ben']);
  });

  it('orders by join date from the control, oldest first, then newest first', () => {
    renderList();
    fireEvent.change(screen.getByTestId('member-sort-field'), { target: { value: 'joined_at' } });
    expect(order()).toEqual(['Cleo', 'Alice', 'Ben']);

    fireEvent.click(screen.getByTestId('member-sort-direction'));
    expect(order()).toEqual(['Ben', 'Alice', 'Cleo']);
    expect(screen.getByTestId('member-sort-direction')).toHaveAttribute('aria-label', 'Descending');
  });

  it('orders by the sharing state from the control', () => {
    renderList();
    fireEvent.change(screen.getByTestId('member-sort-field'), { target: { value: 'peer_sharing_consent' } });
    fireEvent.click(screen.getByTestId('member-sort-direction'));
    expect(order()[0]).toBe('Ben');
  });

  it('shares one state between the headers and the control', () => {
    renderList();
    fireEvent.click(screen.getByRole('columnheader', { name: /Name/ }));
    expect(order()).toEqual(['Alice', 'Ben', 'Cleo']);
    expect(screen.getByTestId('member-sort-field')).toHaveValue('display_name');
  });

  it('labels the control and every field in French', async () => {
    await i18n.changeLanguage('fr');
    try {
      renderList();
      const field = screen.getByTestId('member-sort-field');
      expect(field).toHaveAccessibleName(i18n.t('groups.sortBy'));
      const labels = within(field).getAllByRole('option').map((option) => option.textContent);
      for (const key of ['groups.colName', 'settingsUi.role', 'groups.colJoined', 'groups.peerSharing']) {
        expect(labels).toContain(i18n.t(key));
      }
      // No option or label falls back to its raw key.
      for (const label of [...labels, i18n.t('groups.sortBy'), i18n.t('groups.sortAscending')]) {
        expect(label).not.toMatch(/^(groups|settingsUi)\./);
      }
    } finally {
      await i18n.changeLanguage('en');
    }
  });
});
