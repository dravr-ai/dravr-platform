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
import { useTranslation } from '@pierre/i18n';

interface GroupInfoPanelProps {
  /** The group this conversation is scoped to. */
  groupId: string;
  /**
   * The caller stopped being a member — left, or archived the group they own.
   * The host closes the panel and drops the thread selection.
   */
  onMembershipEnded: () => void;
}

// Built at import time, where `t` does not exist: the table carries the key
// and the render resolves it.
const TREND_DISPLAY: Record<GroupTrend, { labelKey: string; color: string }> = {
  improving: { labelKey: 'groups.improving', color: 'text-success' },
  stable: { labelKey: 'groups.stable', color: 'text-on-surface-variant' },
  declining: { labelKey: 'groups.declining', color: 'text-warning' },
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
  const { t } = useTranslation();
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
        consent ? t('groups.sharingOn') : t('groups.sharingOff'),
        consent
          ? t('groups.dataSharingOnBody')
          : t('groups.sharingOffNotice'),
      );
    } catch (err) {
      const message = err instanceof Error ? err.message : t('groups.consentFailed');
      showError(t('app.updateFailed'), message);
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
      showSuccess(t('app.settingsSaved'), 'Group settings have been updated.');
    } catch (err) {
      const message = err instanceof Error ? err.message : t('groups.saveFailed');
      showError(t('app.saveFailed'), message);
    }
  };

  const handleRemoveCoach = async () => {
    try {
      await removeCoach();
      showSuccess(t('app.coachRemoved'), 'The human coach has been detached from this group.');
      setConfirmRemoveCoach(false);
    } catch (err) {
      const message = err instanceof Error ? err.message : t('groups.removeCoachFailed');
      showError(t('app.removeFailed'), message);
    }
  };

  const handleLeave = async () => {
    try {
      await leaveGroup(groupId);
      showSuccess(t('app.leftGroup'), 'You have left the group.');
      onMembershipEnded();
    } catch (err) {
      const message = err instanceof Error ? err.message : t('groups.leaveFailed');
      showError(t('app.leaveFailed'), message);
    }
  };

  const handleDelete = async () => {
    try {
      await deleteGroup(groupId);
      showSuccess(t('app.groupDeleted'), 'The group has been permanently archived.');
      onMembershipEnded();
    } catch (err) {
      const message = err instanceof Error ? err.message : t('groups.deleteFailed');
      showError(t('app.deleteFailed'), message);
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
        {t('groups.groupLoadFailed')}
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
              {t('groups.owner')}
            </span>
          )}
          {group.coach_user_id && (
            <span className="flex items-center gap-1.5 text-primary">
              <UserCog className="w-3.5 h-3.5" aria-hidden="true" />
              {t('groups.coachAttached')}
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
              label={t('groups.shareMyData')}
              description={
                group.peer_data_sharing
                  ? t('groups.shareMyDataHint')
                  : t('groups.sharingOffHint')
              }
              checked={currentMember.peer_sharing_consent}
              disabled={isSavingConsent}
              onChange={(e) => void handleConsentChange(e.target.checked)}
              data-testid="peer-consent-switch"
            />
          </Card>
        </div>
      )}

      <Section icon={<Users className="w-3.5 h-3.5" aria-hidden="true" />} title={t('groups.tabMembers')}>
        <MemberList
          groupId={groupId}
          members={members}
          currentUserRole={currentUserRole}
          currentUserId={currentUserId}
          isLoading={isMembersLoading}
        />
      </Section>

      <Section icon={<Link2 className="w-3.5 h-3.5" aria-hidden="true" />} title={t('groups.tabInvites')}>
        <InviteManager groupId={groupId} currentUserRole={currentUserRole} />
      </Section>

      {isAdmin && (
        <Section icon={<UserCog className="w-3.5 h-3.5" aria-hidden="true" />} title={t('chat.coachPanelTitle')}>
          <p className="text-sm text-on-surface">
            {t('groups.coachRoomHint')}
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
                {t('groups.removeCoach')}
              </Button>
            </div>
          ) : (
            <p className="text-sm text-outline">
              {t('groups.noCoachAttachedHint')}
            </p>
          )}
        </Section>
      )}

      {isAdmin && (
        <Section icon={<Settings className="w-3.5 h-3.5" aria-hidden="true" />} title={t('groups.tabSettings')}>
          <div className="space-y-4">
            <Input
              label={t('groups.name')}
              variant="dark"
              value={editName}
              onChange={(e) => setEditName(e.target.value)}
              maxLength={100}
            />
            <Textarea
              label={t('chat.descriptionLabel')}
              rows={3}
              value={editDescription}
              onChange={(e) => setEditDescription(e.target.value)}
              maxLength={500}
            />
            <Checkbox
              label={t('groups.peerSharingEnable')}
              description="Allows members who consent to see each other's aggregated training data."
              checked={editPeerSharing}
              onChange={(e) => setEditPeerSharing(e.target.checked)}
            />
            <Select
              id="group-respond-mode"
              label={t('groups.respondMode')}
              value={editRespondMode}
              onChange={(e) => setEditRespondMode(e.target.value as GroupRespondMode)}
              options={[
                { value: 'all', label: t('groups.respondEvery') },
                { value: 'mentions', label: t('groups.respondMentioned') },
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
                {t('groups.saveSettings')}
              </Button>
            </div>
          </div>
        </Section>
      )}

      <Section icon={<BarChart3 className="w-3.5 h-3.5" aria-hidden="true" />} title={t('groups.tabAnalytics')}>
        {isStatsLoading ? (
          <div className="flex justify-center py-6">
            <div className="pierre-spinner" />
          </div>
        ) : stats ? (
          <div className="grid grid-cols-2 gap-3" data-testid="group-info-stats">
            <div className="stat-card-dark">
              <p className="text-xs font-medium text-on-surface-variant mb-1">{t('groups.activeMembers')}</p>
              <p className="text-xl font-bold text-on-surface">{stats.active_members}</p>
              <p className="text-xs text-outline mt-1">of {stats.total_members} total</p>
            </div>
            <div className="stat-card-dark">
              <p className="text-xs font-medium text-on-surface-variant mb-1">{t('groups.avgWeeklyVolume')}</p>
              <p className="text-xl font-bold text-on-surface">
                {stats.avg_weekly_volume_km.toFixed(1)}
                <span className="text-sm text-on-surface-variant ml-1">km</span>
              </p>
            </div>
            <div className="stat-card-dark">
              <p className="text-xs font-medium text-on-surface-variant mb-1">{t('groups.avgCtl')}</p>
              <p className="text-xl font-bold text-on-surface">
                {stats.avg_ctl !== null ? stats.avg_ctl.toFixed(1) : '--'}
              </p>
            </div>
            <div className="stat-card-dark">
              <p className="text-xs font-medium text-on-surface-variant mb-1">{t('groups.flagged')}</p>
              <p className="text-xl font-bold text-on-surface">{stats.flagged_members}</p>
              <p className="text-xs mt-1">
                <span className={TREND_DISPLAY[stats.weekly_trend].color}>
                  {t(TREND_DISPLAY[stats.weekly_trend].labelKey)}
                </span>
              </p>
            </div>
          </div>
        ) : (
          <p className="text-sm text-outline">{t('groups.noStats')}</p>
        )}
        <GroupInsightsPanel groupId={groupId} isAdmin={isAdmin} weeklyDigestEnabled={weeklyDigest} />
      </Section>

      <Section icon={<MessageCircle className="w-3.5 h-3.5" aria-hidden="true" />} title={t('groups.tabRoom')}>
        <GroupTranscriptPanel groupId={groupId} />
      </Section>

      <section className="space-y-3 rounded-lg border border-error/20 p-4">
        <h4 className="text-xs font-semibold uppercase tracking-wide text-error">{t('chat.dangerZone')}</h4>
        {!isOwner && (
          <div className="flex items-center justify-between gap-3">
            <div className="min-w-0">
              <p className="text-sm text-on-surface">{t('groups.leaveGroup')}</p>
              <p className="text-xs text-outline mt-0.5">
                {t('groups.leaveHint')}
              </p>
            </div>
            <Button
              variant="danger"
              size="sm"
              onClick={() => setConfirmLeave(true)}
              data-testid="group-info-leave"
            >
              {t('groups.leave')}
            </Button>
          </div>
        )}
        {isOwner && (
          <div className="flex items-center justify-between gap-3">
            <div className="min-w-0">
              <p className="text-sm text-on-surface">{t('groups.deleteGroup')}</p>
              <p className="text-xs text-outline mt-0.5">
                {t('groups.deleteHint')}
              </p>
            </div>
            <Button
              variant="danger"
              size="sm"
              onClick={() => setConfirmDelete(true)}
              data-testid="group-info-delete"
            >
              {t('groups.deleteGroup')}
            </Button>
          </div>
        )}
      </section>

      <ConfirmDialog
        isOpen={confirmLeave}
        onClose={() => setConfirmLeave(false)}
        onConfirm={() => void handleLeave()}
        title={t('groups.leaveGroup')}
        message={`Are you sure you want to leave "${group.name}"? You will need a new invite to rejoin.`}
        confirmLabel="Leave Group"
        variant="warning"
        isLoading={isLeaving}
      />

      <ConfirmDialog
        isOpen={confirmDelete}
        onClose={() => setConfirmDelete(false)}
        onConfirm={() => void handleDelete()}
        title={t('groups.deleteGroup')}
        message={`This will permanently archive "${group.name}" and remove all members. This action cannot be undone.`}
        confirmLabel="Delete Group"
        variant="danger"
        isLoading={isDeleting}
      />

      <ConfirmDialog
        isOpen={confirmRemoveCoach}
        onClose={() => setConfirmRemoveCoach(false)}
        onConfirm={() => void handleRemoveCoach()}
        title={t('groups.removeCoach')}
        message="Detach the human coach from this group? They will lose access to the group's roster. You can invite a coach again later."
        confirmLabel="Remove Coach"
        variant="warning"
        isLoading={isRemovingCoach}
      />
    </div>
  );
}
