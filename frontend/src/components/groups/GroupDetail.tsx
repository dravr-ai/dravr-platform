// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Group detail page with tabbed navigation for members, stats, invites, settings
// ABOUTME: Displays group header info and delegates to sub-components for each tab

import { useState } from 'react';
import { ArrowLeft, Users, BarChart3, Link2, Settings, Crown } from 'lucide-react';
import { useGroup, useGroupMembers, useGroupStats, useUpdateGroup, useLeaveGroup, useDeleteGroup } from '../../hooks/useGroups';
import { useAuth } from '../../hooks/useAuth';
import { Card, Button, Tabs, TabPanel, Input, ConfirmDialog, useErrorToast, useSuccessToast } from '../ui';
import MemberList from './MemberList';
import InviteManager from './InviteManager';
import type { GroupRole, GroupTrend } from '@pierre/shared-types';

interface GroupDetailProps {
  groupId: string;
  onBack: () => void;
}

const DETAIL_TABS = [
  {
    id: 'members',
    label: 'Members',
    icon: <Users className="w-4 h-4" />,
  },
  {
    id: 'stats',
    label: 'Stats',
    icon: <BarChart3 className="w-4 h-4" />,
  },
  {
    id: 'invites',
    label: 'Invites',
    icon: <Link2 className="w-4 h-4" />,
  },
  {
    id: 'settings',
    label: 'Settings',
    icon: <Settings className="w-4 h-4" />,
  },
];

const TREND_DISPLAY: Record<GroupTrend, { label: string; color: string }> = {
  improving: { label: 'Improving', color: 'text-emerald-400' },
  stable: { label: 'Stable', color: 'text-on-surface-variant' },
  declining: { label: 'Declining', color: 'text-amber-400' },
};

export default function GroupDetail({ groupId, onBack }: GroupDetailProps) {
  const { group, isLoading: isGroupLoading } = useGroup(groupId);
  const { members, isLoading: isMembersLoading } = useGroupMembers(groupId);
  const { stats, isLoading: isStatsLoading } = useGroupStats(groupId);
  const { updateGroup, isPending: isUpdating } = useUpdateGroup(groupId);
  const { leaveGroup, isPending: isLeaving } = useLeaveGroup();
  const { deleteGroup, isPending: isDeleting } = useDeleteGroup();
  const auth = useAuth();
  const showError = useErrorToast();
  const showSuccess = useSuccessToast();

  const [activeTab, setActiveTab] = useState('members');
  const [confirmLeave, setConfirmLeave] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);

  // Settings form state
  const [editName, setEditName] = useState('');
  const [editDescription, setEditDescription] = useState('');
  const [editPeerSharing, setEditPeerSharing] = useState(false);
  const [settingsInitialized, setSettingsInitialized] = useState(false);

  // Initialize settings form from group data once loaded
  if (group && !settingsInitialized) {
    setEditName(group.name);
    setEditDescription(group.description ?? '');
    setEditPeerSharing(group.peer_data_sharing);
    setSettingsInitialized(true);
  }

  const currentUserId = auth.user?.id ?? '';
  const currentMember = members.find((m) => m.user_id === currentUserId);
  const currentUserRole: GroupRole = currentMember?.role ?? 'member';
  const isOwner = currentUserRole === 'owner';
  const isAdmin = currentUserRole === 'admin' || isOwner;

  const handleSaveSettings = async () => {
    if (!group) return;
    try {
      await updateGroup({
        name: editName.trim() || undefined,
        description: editDescription.trim() || undefined,
        peer_data_sharing: editPeerSharing,
      });
      showSuccess('Settings saved', 'Group settings have been updated.');
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to save settings';
      showError('Save failed', message);
    }
  };

  const handleLeave = async () => {
    try {
      await leaveGroup(groupId);
      showSuccess('Left group', 'You have left the group.');
      onBack();
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to leave group';
      showError('Leave failed', message);
    }
  };

  const handleDelete = async () => {
    try {
      await deleteGroup(groupId);
      showSuccess('Group deleted', 'The group has been permanently archived.');
      onBack();
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to delete group';
      showError('Delete failed', message);
    }
  };

  if (isGroupLoading) {
    return (
      <div className="flex justify-center py-12">
        <div className="pierre-spinner" />
      </div>
    );
  }

  if (!group) {
    return (
      <Card variant="dark" className="!p-10 text-center">
        <p className="text-on-surface-variant">Group not found.</p>
        <Button variant="secondary" onClick={onBack} className="mt-4">
          Back to Groups
        </Button>
      </Card>
    );
  }

  return (
    <div className="space-y-6">
      {/* Back button + header */}
      <div>
        <button
          onClick={onBack}
          className="flex items-center gap-1.5 text-on-surface-variant hover:text-pierre-violet transition-colors mb-4"
        >
          <ArrowLeft className="w-4 h-4" />
          <span className="text-sm">All Groups</span>
        </button>

        <div className="flex items-start justify-between gap-4">
          <div className="flex-1 min-w-0">
            <div className="flex items-center gap-3 mb-1">
              <h2 className="text-2xl font-bold text-on-surface truncate">{group.name}</h2>
              {!group.is_active && (
                <span className="px-2 py-0.5 text-xs font-medium bg-zinc-700/50 text-on-surface-variant rounded-full">
                  Inactive
                </span>
              )}
            </div>
            {group.description && (
              <p className="text-on-surface-variant text-sm mt-1">{group.description}</p>
            )}
            <div className="flex items-center gap-4 text-sm text-outline mt-3">
              <span className="flex items-center gap-1.5">
                <Users className="w-4 h-4" />
                {members.length} {members.length === 1 ? 'member' : 'members'}
              </span>
              {isOwner && (
                <span className="flex items-center gap-1.5 text-amber-400">
                  <Crown className="w-4 h-4" />
                  Owner
                </span>
              )}
            </div>
          </div>
        </div>
      </div>

      {/* Tab navigation */}
      <Tabs
        tabs={DETAIL_TABS}
        activeTab={activeTab}
        onChange={setActiveTab}
        variant="pills"
      />

      {/* Members tab */}
      <TabPanel id="members" activeTab={activeTab}>
        <Card variant="dark" className="!p-5">
          <MemberList
            groupId={groupId}
            members={members}
            currentUserRole={currentUserRole}
            currentUserId={currentUserId}
            isLoading={isMembersLoading}
          />
        </Card>
      </TabPanel>

      {/* Stats tab */}
      <TabPanel id="stats" activeTab={activeTab}>
        {isStatsLoading ? (
          <div className="flex justify-center py-8">
            <div className="pierre-spinner" />
          </div>
        ) : stats ? (
          <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
            <div className="stat-card-dark">
              <p className="text-xs font-medium text-on-surface-variant mb-1">Active Members</p>
              <p className="text-2xl font-bold text-on-surface">{stats.active_members}</p>
              <p className="text-xs text-outline mt-1">of {stats.total_members} total</p>
            </div>
            <div className="stat-card-dark">
              <p className="text-xs font-medium text-on-surface-variant mb-1">Avg Weekly Volume</p>
              <p className="text-2xl font-bold text-on-surface">
                {stats.avg_weekly_volume_km.toFixed(1)}
                <span className="text-sm text-on-surface-variant ml-1">km</span>
              </p>
            </div>
            <div className="stat-card-dark">
              <p className="text-xs font-medium text-on-surface-variant mb-1">Avg CTL</p>
              <p className="text-2xl font-bold text-on-surface">
                {stats.avg_ctl !== null ? stats.avg_ctl.toFixed(1) : '--'}
              </p>
            </div>
            <div className="stat-card-dark">
              <p className="text-xs font-medium text-on-surface-variant mb-1">Flagged</p>
              <p className="text-2xl font-bold text-on-surface">{stats.flagged_members}</p>
              <p className="text-xs mt-1">
                <span className={TREND_DISPLAY[stats.weekly_trend].color}>
                  {TREND_DISPLAY[stats.weekly_trend].label}
                </span>
              </p>
            </div>
          </div>
        ) : (
          <Card variant="dark" className="!p-8 text-center">
            <BarChart3 className="w-8 h-8 text-on-surface-variant mx-auto mb-3" />
            <p className="text-outline">No stats available yet.</p>
          </Card>
        )}
      </TabPanel>

      {/* Invites tab */}
      <TabPanel id="invites" activeTab={activeTab}>
        <Card variant="dark" className="!p-5">
          <InviteManager groupId={groupId} currentUserRole={currentUserRole} />
        </Card>
      </TabPanel>

      {/* Settings tab */}
      <TabPanel id="settings" activeTab={activeTab}>
        <div className="space-y-6">
          {/* Edit group settings (admin/owner) */}
          {isAdmin && (
            <Card variant="dark" className="!p-5">
              <h4 className="text-sm font-semibold text-on-surface mb-4">Group Settings</h4>
              <div className="space-y-4">
                <Input
                  label="Group Name"
                  variant="dark"
                  value={editName}
                  onChange={(e) => setEditName(e.target.value)}
                  maxLength={100}
                />
                <div className="w-full">
                  <label className="block text-sm font-medium text-on-surface mb-1.5">
                    Description
                  </label>
                  <textarea
                    className="w-full px-4 py-2.5 text-sm bg-surface-container-low text-on-surface placeholder:text-outline border ghost-border rounded-lg transition-all focus:outline-none focus:ring-2 focus:ring-pierre-violet focus:ring-opacity-30 focus:border-pierre-violet resize-none"
                    rows={3}
                    value={editDescription}
                    onChange={(e) => setEditDescription(e.target.value)}
                    maxLength={500}
                  />
                </div>
                <label className="flex items-center gap-3 cursor-pointer group">
                  <input
                    type="checkbox"
                    checked={editPeerSharing}
                    onChange={(e) => setEditPeerSharing(e.target.checked)}
                    className="w-4 h-4 rounded ghost-border bg-surface-container-low text-pierre-violet focus:ring-pierre-violet focus:ring-offset-0"
                  />
                  <div>
                    <span className="text-sm text-on-surface group-hover:text-on-surface transition-colors">
                      Enable peer data sharing
                    </span>
                    <p className="text-xs text-outline mt-0.5">
                      Allows members who consent to see each other's aggregated training data.
                    </p>
                  </div>
                </label>
                <div className="flex justify-end">
                  <Button variant="primary" onClick={handleSaveSettings} loading={isUpdating}>
                    Save Settings
                  </Button>
                </div>
              </div>
            </Card>
          )}

          {/* Danger zone */}
          <Card variant="dark" className="!p-5 border border-red-500/20">
            <h4 className="text-sm font-semibold text-red-400 mb-4">Danger Zone</h4>
            <div className="space-y-4">
              {/* Leave group (non-owners) */}
              {!isOwner && (
                <div className="flex items-center justify-between">
                  <div>
                    <p className="text-sm text-on-surface">Leave Group</p>
                    <p className="text-xs text-outline mt-0.5">
                      You will lose access to this group and its data.
                    </p>
                  </div>
                  <Button variant="danger" size="sm" onClick={() => setConfirmLeave(true)}>
                    Leave
                  </Button>
                </div>
              )}

              {/* Delete group (owners only) */}
              {isOwner && (
                <div className="flex items-center justify-between">
                  <div>
                    <p className="text-sm text-on-surface">Delete Group</p>
                    <p className="text-xs text-outline mt-0.5">
                      Permanently archive this group. All members will be removed.
                    </p>
                  </div>
                  <Button variant="danger" size="sm" onClick={() => setConfirmDelete(true)}>
                    Delete Group
                  </Button>
                </div>
              )}
            </div>
          </Card>
        </div>
      </TabPanel>

      {/* Confirm leave dialog */}
      <ConfirmDialog
        isOpen={confirmLeave}
        onClose={() => setConfirmLeave(false)}
        onConfirm={handleLeave}
        title="Leave Group"
        message={`Are you sure you want to leave "${group.name}"? You will need a new invite to rejoin.`}
        confirmLabel="Leave Group"
        variant="warning"
        isLoading={isLeaving}
      />

      {/* Confirm delete dialog */}
      <ConfirmDialog
        isOpen={confirmDelete}
        onClose={() => setConfirmDelete(false)}
        onConfirm={handleDelete}
        title="Delete Group"
        message={`This will permanently archive "${group.name}" and remove all members. This action cannot be undone.`}
        confirmLabel="Delete Group"
        variant="danger"
        isLoading={isDeleting}
      />
    </div>
  );
}
