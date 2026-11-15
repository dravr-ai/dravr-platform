// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the group member rows: the sort control only above eight people, every field it orders by, and the `⋯` menu
// ABOUTME: Red if the join date or the sharing state can no longer order the list (carnet#1191), or a removal skips its confirm

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { GroupMember } from '@pierre/shared-types';
import { i18n } from '@pierre/i18n';
import MemberList from '../MemberList';
import { ToastProvider } from '../../ui';

const removeMember = vi.fn();
const updateRole = vi.fn();

vi.mock('../../../hooks/useGroups', () => ({
  useRemoveMember: () => ({ removeMember, isPending: false }),
  useUpdateMemberRole: () => ({ updateRole, isPending: false }),
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
const TRIO: GroupMember[] = [
  member({ user_id: 'alice', display_name: 'Alice', role: 'owner', joined_at: '2026-03-01T00:00:00Z' }),
  member({ user_id: 'ben', display_name: 'Ben', joined_at: '2026-05-01T00:00:00Z', peer_sharing_consent: true }),
  member({ user_id: 'cleo', display_name: 'Cleo', role: 'admin', joined_at: '2026-01-15T00:00:00Z' }),
];

// Nine people: the trio plus six who joined after them, in name order.
const CLUB: GroupMember[] = [
  ...TRIO,
  ...['Dora', 'Eli', 'Fay', 'Gus', 'Hana', 'Ivo'].map((name, index) =>
    member({ user_id: name.toLowerCase(), display_name: name, joined_at: `2026-06-0${index + 1}T00:00:00Z` }),
  ),
];

function renderList(members: GroupMember[] = TRIO, currentUserRole: GroupMember['role'] = 'owner') {
  return render(
    <ToastProvider>
      <MemberList
        groupId="group-1"
        members={members}
        currentUserRole={currentUserRole}
        currentUserId="alice"
        isLoading={false}
      />
    </ToastProvider>,
  );
}

/** The member names, top to bottom. */
function order(): string[] {
  const list = within(screen.getByTestId('member-list')).getByRole('list');
  return within(list)
    .getAllByRole('listitem')
    .map((row) => row.querySelector('[title]')?.textContent ?? '');
}

describe('MemberList', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    removeMember.mockResolvedValue(undefined);
    updateRole.mockResolvedValue(undefined);
  });

  it('draws one row per person with their role and sharing state, and no sort control for a small group', () => {
    renderList();

    expect(order()).toEqual(['Alice', 'Cleo', 'Ben']);
    expect(screen.queryByTestId('member-sort')).toBeNull();
    expect(screen.getByTestId('member-detail-ben')).toHaveTextContent('Member·Sharing on');
    expect(screen.getByTestId('member-detail-cleo')).toHaveTextContent('Admin·Sharing off');
    // No table, no filled red on any row.
    expect(screen.queryByRole('table')).toBeNull();
    expect(document.querySelector('.btn-danger')).toBeNull();
  });

  it('offers the sort control above eight people, and orders by role until asked otherwise', () => {
    renderList(CLUB);

    expect(screen.getByTestId('member-sort')).toBeInTheDocument();
    expect(order().slice(0, 3)).toEqual(['Alice', 'Cleo', 'Ben']);
  });

  it('orders by join date from the control, oldest first, then newest first', () => {
    renderList(CLUB);
    fireEvent.change(screen.getByTestId('member-sort-field'), { target: { value: 'joined_at' } });
    expect(order().slice(0, 3)).toEqual(['Cleo', 'Alice', 'Ben']);

    fireEvent.click(screen.getByTestId('member-sort-direction'));
    expect(order().slice(-3)).toEqual(['Ben', 'Alice', 'Cleo']);
    expect(screen.getByTestId('member-sort-direction')).toHaveAttribute('aria-label', 'Descending');
  });

  it('orders by the sharing state from the control', () => {
    renderList(CLUB);
    fireEvent.change(screen.getByTestId('member-sort-field'), { target: { value: 'peer_sharing_consent' } });
    fireEvent.click(screen.getByTestId('member-sort-direction'));
    expect(order()[0]).toBe('Ben');
  });

  it('labels the control and every field in French', async () => {
    await i18n.changeLanguage('fr');
    try {
      renderList(CLUB);
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

  it("puts role and removal behind each other member's ⋯, and removes only after the confirm", async () => {
    const user = userEvent.setup();
    renderList();

    // The viewer's own row and the owner's carry no menu.
    expect(screen.queryByTestId('member-menu-alice')).toBeNull();
    await user.click(screen.getByRole('button', { name: 'More actions for Ben' }));
    await user.click(screen.getByRole('menuitem', { name: 'Promote to admin' }));
    await waitFor(() => expect(updateRole).toHaveBeenCalledWith({ userId: 'ben', role: 'admin' }));

    await user.click(screen.getByRole('button', { name: 'More actions for Cleo' }));
    await user.click(screen.getByRole('menuitem', { name: 'Remove from group…' }));
    expect(removeMember).not.toHaveBeenCalled();
    const confirm = await screen.findByRole('dialog');
    expect(confirm).toHaveTextContent('Remove Cleo?');
    await user.click(within(confirm).getByRole('button', { name: 'Remove' }));
    await waitFor(() => expect(removeMember).toHaveBeenCalledWith('cleo'));
  });

  it('closes the menu on Escape and hands focus back to its trigger', async () => {
    const user = userEvent.setup();
    renderList();

    const trigger = screen.getByRole('button', { name: 'More actions for Ben' });
    await user.click(trigger);
    expect(screen.getByRole('menu')).toBeInTheDocument();
    expect(document.activeElement).toBe(screen.getByRole('menuitem', { name: 'Promote to admin' }));
    await user.keyboard('{ArrowDown}');
    expect(document.activeElement).toBe(screen.getByRole('menuitem', { name: 'Remove from group…' }));

    await user.keyboard('{Escape}');
    expect(screen.queryByRole('menu')).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });

  it('gives an admin removal but not the role change, and a plain member no menu at all', () => {
    const { unmount } = renderList(TRIO, 'admin');
    expect(screen.getByTestId('member-menu-ben')).toBeInTheDocument();
    unmount();

    renderList(TRIO, 'member');
    expect(screen.queryByRole('button', { name: /More actions/ })).toBeNull();
  });
});
