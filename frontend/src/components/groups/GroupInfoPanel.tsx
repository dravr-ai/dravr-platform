// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Group info for the open group thread — members, invites, coach, settings, analytics, transcript
// ABOUTME: The Groups tab's management surface, re-homed where App Messaging keeps it: inside the chat

import { useState } from 'react';
import { BarChart3, Crown, Link2, MessageCircle, Settings, UserCog, Users } from 'lucide-react';
import {
  useGroup,
  useGroupMembers,
  useGroupPermissions,
  useGroupStats,
  useUpdateGroup,
  useUpdatePeerConsent,
  useLeaveGroup,
  useDeleteGroup,
  useRemoveCoach,
} from '../../hooks/useGroups';
import { useAuth } from '../../hooks/useAuth';
import {
  Button,
  Card,
  Checkbox,
  ConfirmDialog,
  Input,
  Select,
  Textarea,
  useErrorToast,
  useSuccessToast,
} from '../ui';
import MemberList from './MemberList';
import InviteManager from './InviteManager';
import GroupInsightsPanel from './GroupInsightsPanel';
import GroupTranscriptPanel from './GroupTranscriptPanel';
import type { GroupRespondMode, GroupRole, GroupTrend } from '@pierre/shared-types';

interface GroupInfoPanelProps {
  /** The group this conversation is scoped to. */
  groupId: string;
  /**
   * The caller stopped being a member — left, or archived the group they own.
   * The host closes the panel and drops the thread selection.
   */
  onMembershipEnded: () => void;
}

const TREND_DISPLAY: Record<GroupTrend, { label: string; color: string }> = {
  improving: { label: 'Improving', color: 'text-success' },
  stable: { label: 'Stable', color: 'text-on-surface-variant' },
  declining: { label: 'Declining', color: 'text-warning' },
};

/** One titled block of the panel. */
function Section({
  icon,
  title,
  children,
}: {
  icon: React.ReactNode;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section className="space-y-2">
      <h4 className="flex items-center gap-1.5 text-xs font-semibold uppercase tracking-wide text-outline">
        {icon}
        {title}
      </h4>
      {children}
    </section>
  );
}

/**
 * Everything the Groups tab used to hold, for the group behind the open thread.
 *
 * Tapping the thread header is how Telegram and WhatsApp reach group info, and
 * it is now the only way here: the roster and its admin actions, the invite
 * links, the human coach, the group settings, the caller's own peer-sharing
 * consent, the analytics an admin may read, the shared room transcript, and
 * the two exits. Creating and joining are commands, so neither appears.
 */
export default function GroupInfoPanel({ groupId, onMembershipEnded }: GroupInfoPanelProps) {
  const { group, isLoading: isGroupLoading } = useGroup(groupId);
  const { members, isLoading: isMembersLoading } = useGroupMembers(groupId);
  const { stats, isLoading: isStatsLoading } = useGroupStats(groupId);
  const { weeklyDigest } = useGroupPermissions();
  const { updateGroup, isPending: isUpdating } = useUpdateGroup(groupId);
  const { updateConsent, isPending: isSavingConsent } = useUpdatePeerConsent(groupId);
  const { leaveGroup, isPending: isLeaving } = useLeaveGroup();
  const { deleteGroup, isPending: isDeleting } = useDeleteGroup();
  const { removeCoach, isPending: isRemovingCoach } = useRemoveCoach(groupId);
  const auth = useAuth();
  const showError = useErrorToast();
  const showSuccess = useSuccessToast();

  const [confirmLeave, setConfirmLeave] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [confirmRemoveCoach, setConfirmRemoveCoach] = useState(false);

  const [editName, setEditName] = useState('');
  const [editDescription, setEditDescription] = useState('');
  const [editPeerSharing, setEditPeerSharing] = useState(false);
  const [editRespondMode, setEditRespondMode] = useState<GroupRespondMode>('all');
  const [settingsInitialized, setSettingsInitialized] = useState(false);

  // Seed the settings form from the group the first time it resolves.
  if (group && !settingsInitialized) {
    setEditName(group.name);
    setEditDescription(group.description ?? '');
    setEditPeerSharing(group.peer_data_sharing);
    setEditRespondMode(group.respond_mode ?? 'all');
    setSettingsInitialized(true);
  }

  const currentUserId = auth.user?.id ?? '';
  const currentMember = members.find((m) => m.user_id === currentUserId);
  const currentUserRole: GroupRole = currentMember?.role ?? 'member';
  const isOwner = currentUserRole === 'owner';
  const isAdmin = currentUserRole === 'admin' || isOwner;

  /**
   * Set the caller's own peer-sharing consent. The route writes the caller's
   * membership row and nothing else, so this switch is bound to
   * `currentMember` — never to a row picked off the roster.
   */
  const handleConsentChange = async (consent: boolean) => {
    try {
      await updateConsent(consent);
      showSuccess(
        consent ? 'Sharing on' : 'Sharing off',
        consent
          ? 'Your training data is now readable by this group.'
          : 'Your training data is no longer readable by this group.',
      );
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to update sharing consent';
      showError('Update failed', message);
    }
  };

  const handleSaveSettings = async () => {
    if (!group) return;
    try {
      await updateGroup({
        name: editName.trim() || undefined,
        description: editDescription.trim() || undefined,
        peer_data_sharing: editPeerSharing,
        respond_mode: editRespondMode,
      });
      showSuccess('Settings saved', 'Group settings have been updated.');
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to save settings';
      showError('Save failed', message);
    }
  };

  const handleRemoveCoach = async () => {
    try {
      await removeCoach();
      showSuccess('Coach removed', 'The human coach has been detached from this group.');
      setConfirmRemoveCoach(false);
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to remove coach';
      showError('Remove failed', message);
    }
  };

  const handleLeave = async () => {
    try {
      await leaveGroup(groupId);
      showSuccess('Left group', 'You have left the group.');
      onMembershipEnded();
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to leave group';
      showError('Leave failed', message);
    }
  };

  const handleDelete = async () => {
    try {
      await deleteGroup(groupId);
      showSuccess('Group deleted', 'The group has been permanently archived.');
      onMembershipEnded();
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
      <p className="py-10 text-center text-sm text-outline" data-testid="group-info-missing">
        This group could not be loaded.
      </p>
    );
  }

  return (
    <div className="space-y-6" data-testid="group-info-panel">
      <section>
        <h3 className="text-lg font-semibold text-on-surface" data-testid="group-info-name">
          {group.name}
        </h3>
        {group.description && (
          <p
            className="mt-1 text-sm text-on-surface-variant"
            data-testid="group-info-description"
          >
            {group.description}
          </p>
        )}
        <div className="mt-2 flex flex-wrap items-center gap-3 text-xs text-outline">
          <span className="flex items-center gap-1.5">
            <Users className="w-3.5 h-3.5" aria-hidden="true" />
            {members.length} {members.length === 1 ? 'member' : 'members'}
          </span>
          {isOwner && (
            <span className="flex items-center gap-1.5 text-warning">
              <Crown className="w-3.5 h-3.5" aria-hidden="true" />
              Owner
            </span>
          )}
          {group.coach_user_id && (
            <span className="flex items-center gap-1.5 text-primary">
              <UserCog className="w-3.5 h-3.5" aria-hidden="true" />
              Coach attached
            </span>
          )}
        </div>
      </section>

      {/* The caller's own peer-sharing consent. The group can allow peer
          sharing, but each athlete still decides whether their own training
          data is part of it. */}
      {currentMember && (
        <div data-testid="peer-consent-card">
          <Card variant="dark" className="!p-4">
            <Checkbox
              label="Share my training data with this group"
              description={
                group.peer_data_sharing
                  ? 'Lets the coach compare you with the rest of the group. Applies to your membership only.'
                  : 'Group sharing is off, so your data stays private either way.'
              }
              checked={currentMember.peer_sharing_consent}
              disabled={isSavingConsent}
              onChange={(e) => void handleConsentChange(e.target.checked)}
              data-testid="peer-consent-switch"
            />
          </Card>
        </div>
      )}

      <Section icon={<Users className="w-3.5 h-3.5" aria-hidden="true" />} title="Members">
        <MemberList
          groupId={groupId}
          members={members}
          currentUserRole={currentUserRole}
          currentUserId={currentUserId}
          isLoading={isMembersLoading}
        />
      </Section>

      <Section icon={<Link2 className="w-3.5 h-3.5" aria-hidden="true" />} title="Invites">
        <InviteManager groupId={groupId} currentUserRole={currentUserRole} />
      </Section>

      {isAdmin && (
        <Section icon={<UserCog className="w-3.5 h-3.5" aria-hidden="true" />} title="Coach">
          <p className="text-sm text-on-surface">
            The AI coach answers in this room; a human coach can oversee it too.
          </p>
          {group.coach_user_id ? (
            <div className="flex items-center justify-between gap-3">
              <code className="min-w-0 flex-1 truncate font-mono text-xs text-outline">
                {group.coach_user_id}
              </code>
              <Button
                variant="danger"
                size="sm"
                onClick={() => setConfirmRemoveCoach(true)}
                data-testid="group-info-remove-coach"
              >
                Remove Coach
              </Button>
            </div>
          ) : (
            <p className="text-sm text-outline">
              No human coach attached. Create a Coach invite above and share it with the coach you
              want to oversee this group.
            </p>
          )}
        </Section>
      )}

      {isAdmin && (
        <Section icon={<Settings className="w-3.5 h-3.5" aria-hidden="true" />} title="Settings">
          <div className="space-y-4">
            <Input
              label="Group Name"
              variant="dark"
              value={editName}
              onChange={(e) => setEditName(e.target.value)}
              maxLength={100}
            />
            <Textarea
              label="Description"
              rows={3}
              value={editDescription}
              onChange={(e) => setEditDescription(e.target.value)}
              maxLength={500}
            />
            <Checkbox
              label="Enable peer data sharing"
              description="Allows members who consent to see each other's aggregated training data."
              checked={editPeerSharing}
              onChange={(e) => setEditPeerSharing(e.target.checked)}
            />
            <Select
              id="group-respond-mode"
              label="Coach replies in the group chat"
              value={editRespondMode}
              onChange={(e) => setEditRespondMode(e.target.value as GroupRespondMode)}
              options={[
                { value: 'all', label: 'To every message' },
                { value: 'mentions', label: 'Only when mentioned' },
              ]}
              helpText={'"Only when mentioned" keeps the coach quiet unless someone @-mentions it or replies to one of its messages; it still follows the discussion for context.'}
            />
            <div className="flex justify-end">
              <Button
                variant="primary"
                onClick={() => void handleSaveSettings()}
                loading={isUpdating}
                data-testid="group-info-save-settings"
              >
                Save Settings
              </Button>
            </div>
          </div>
        </Section>
      )}

      <Section icon={<BarChart3 className="w-3.5 h-3.5" aria-hidden="true" />} title="Analytics">
        {isStatsLoading ? (
          <div className="flex justify-center py-6">
            <div className="pierre-spinner" />
          </div>
        ) : stats ? (
          <div className="grid grid-cols-2 gap-3" data-testid="group-info-stats">
            <div className="stat-card-dark">
              <p className="text-xs font-medium text-on-surface-variant mb-1">Active Members</p>
              <p className="text-xl font-bold text-on-surface">{stats.active_members}</p>
              <p className="text-xs text-outline mt-1">of {stats.total_members} total</p>
            </div>
            <div className="stat-card-dark">
              <p className="text-xs font-medium text-on-surface-variant mb-1">Avg Weekly Volume</p>
              <p className="text-xl font-bold text-on-surface">
                {stats.avg_weekly_volume_km.toFixed(1)}
                <span className="text-sm text-on-surface-variant ml-1">km</span>
              </p>
            </div>
            <div className="stat-card-dark">
              <p className="text-xs font-medium text-on-surface-variant mb-1">Avg CTL</p>
              <p className="text-xl font-bold text-on-surface">
                {stats.avg_ctl !== null ? stats.avg_ctl.toFixed(1) : '--'}
              </p>
            </div>
            <div className="stat-card-dark">
              <p className="text-xs font-medium text-on-surface-variant mb-1">Flagged</p>
              <p className="text-xl font-bold text-on-surface">{stats.flagged_members}</p>
              <p className="text-xs mt-1">
                <span className={TREND_DISPLAY[stats.weekly_trend].color}>
                  {TREND_DISPLAY[stats.weekly_trend].label}
                </span>
              </p>
            </div>
          </div>
        ) : (
          <p className="text-sm text-outline">No stats available yet.</p>
        )}
        <GroupInsightsPanel groupId={groupId} isAdmin={isAdmin} weeklyDigestEnabled={weeklyDigest} />
      </Section>

      <Section icon={<MessageCircle className="w-3.5 h-3.5" aria-hidden="true" />} title="Room">
        <GroupTranscriptPanel groupId={groupId} />
      </Section>

      <section className="space-y-3 rounded-lg border border-error/20 p-4">
        <h4 className="text-xs font-semibold uppercase tracking-wide text-error">Danger Zone</h4>
        {!isOwner && (
          <div className="flex items-center justify-between gap-3">
            <div className="min-w-0">
              <p className="text-sm text-on-surface">Leave Group</p>
              <p className="text-xs text-outline mt-0.5">
                You will lose access to this group and its data.
              </p>
            </div>
            <Button
              variant="danger"
              size="sm"
              onClick={() => setConfirmLeave(true)}
              data-testid="group-info-leave"
            >
              Leave
            </Button>
          </div>
        )}
        {isOwner && (
          <div className="flex items-center justify-between gap-3">
            <div className="min-w-0">
              <p className="text-sm text-on-surface">Delete Group</p>
              <p className="text-xs text-outline mt-0.5">
                Permanently archive this group. All members will be removed.
              </p>
            </div>
            <Button
              variant="danger"
              size="sm"
              onClick={() => setConfirmDelete(true)}
              data-testid="group-info-delete"
            >
              Delete Group
            </Button>
          </div>
        )}
      </section>

      <ConfirmDialog
        isOpen={confirmLeave}
        onClose={() => setConfirmLeave(false)}
        onConfirm={() => void handleLeave()}
        title="Leave Group"
        message={`Are you sure you want to leave "${group.name}"? You will need a new invite to rejoin.`}
        confirmLabel="Leave Group"
        variant="warning"
        isLoading={isLeaving}
      />

      <ConfirmDialog
        isOpen={confirmDelete}
        onClose={() => setConfirmDelete(false)}
        onConfirm={() => void handleDelete()}
        title="Delete Group"
        message={`This will permanently archive "${group.name}" and remove all members. This action cannot be undone.`}
        confirmLabel="Delete Group"
        variant="danger"
        isLoading={isDeleting}
      />

      <ConfirmDialog
        isOpen={confirmRemoveCoach}
        onClose={() => setConfirmRemoveCoach(false)}
        onConfirm={() => void handleRemoveCoach()}
        title="Remove Coach"
        message="Detach the human coach from this group? They will lose access to the group's roster. You can invite a coach again later."
        confirmLabel="Remove Coach"
        variant="warning"
        isLoading={isRemovingCoach}
      />
    </div>
  );
}
