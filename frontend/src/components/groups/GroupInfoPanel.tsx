// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Group info for the open group thread — members, TrainingPeaks links, invites, coach, settings, analytics, room
// ABOUTME: The Groups tab's management surface, re-homed where App Messaging keeps it: inside the chat

import { useState } from 'react';
import { Activity, BarChart3, Crown, Link2, MessageCircle, Settings, UserCog, Users } from 'lucide-react';
import {
  useDelegatedConnections,
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
import DelegatedConnectionsSection from './DelegatedConnectionsSection';
import type { GroupDigestMode, GroupRespondMode, GroupRole, GroupTrend } from '@pierre/shared-types';
import { oneDecimal } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { describeApiError } from '@pierre/ui-logic';

interface GroupInfoPanelProps {
  /** The group this conversation is scoped to. */
  groupId: string;
  /**
   * The caller stopped being a member — left, or archived the group they own.
   * The host closes the panel and drops the thread selection.
   */
  onMembershipEnded: () => void;
  /** Open the connections pane, where a coach connects their own TrainingPeaks. */
  onOpenConnections?: () => void;
}

// Built at import time, where `t` does not exist: the table carries the key
// and the render resolves it.
const TREND_DISPLAY: Record<GroupTrend, { labelKey: string; color: string }> = {
  improving: { labelKey: 'groups.improving', color: 'text-success' },
  stable: { labelKey: 'groups.stable', color: 'text-on-surface-variant' },
  declining: { labelKey: 'groups.declining', color: 'text-warning' },
};

// Each weekly-digest mode's option label and the one-line hint shown under the
// select while it is chosen.
const DIGEST_MODES: Record<GroupDigestMode, { labelKey: string; hintKey: string }> = {
  off: { labelKey: 'groups.digestOff', hintKey: 'groups.digestOffHint' },
  chat: { labelKey: 'groups.digestChat', hintKey: 'groups.digestChatHint' },
  managers: { labelKey: 'groups.digestManagers', hintKey: 'groups.digestManagersHint' },
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
      <h4 className="flex items-center gap-1.5 text-xs font-semibold text-outline">
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
export default function GroupInfoPanel({
  groupId,
  onMembershipEnded,
  onOpenConnections,
}: GroupInfoPanelProps) {
  const { t, language } = useTranslation();
  const auth = useAuth();
  const { group, isLoading: isGroupLoading } = useGroup(groupId);
  const { members, isLoading: isMembersLoading } = useGroupMembers(groupId);

  const currentUserId = auth.user?.id ?? '';
  const currentMember = members.find((m) => m.user_id === currentUserId);
  // The group's human coach holds no membership row: they read the group and
  // its members, link TrainingPeaks athletes, and follow the room, while the
  // member-only surfaces (consent, invites, settings, analytics, leave) stay
  // off, since their routes refuse a non-member.
  const isGroupCoach = !!group?.coach_user_id && group.coach_user_id === currentUserId;
  const isCoachViewer = isGroupCoach && !currentMember;

  const { stats, isLoading: isStatsLoading } = useGroupStats(
    groupId,
    // Held until the group resolves: before then a coach reads as a member.
    !!group && !isCoachViewer,
  );
  const { connections: delegatedConnections, viewer: delegationViewer } = useDelegatedConnections(
    groupId,
    !!group?.coach_user_id,
  );
  const { weeklyDigest } = useGroupPermissions();
  const { updateGroup, isPending: isUpdating } = useUpdateGroup(groupId);
  const { updateConsent, isPending: isSavingConsent } = useUpdatePeerConsent(groupId);
  const { leaveGroup, isPending: isLeaving } = useLeaveGroup();
  const { deleteGroup, isPending: isDeleting } = useDeleteGroup();
  const { removeCoach, isPending: isRemovingCoach } = useRemoveCoach(groupId);
  const showError = useErrorToast();
  const showSuccess = useSuccessToast();

  const [confirmLeave, setConfirmLeave] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [confirmRemoveCoach, setConfirmRemoveCoach] = useState(false);

  const [editName, setEditName] = useState('');
  const [editDescription, setEditDescription] = useState('');
  const [editPeerSharing, setEditPeerSharing] = useState(false);
  const [editRespondMode, setEditRespondMode] = useState<GroupRespondMode>('all');
  const [editDigestMode, setEditDigestMode] = useState<GroupDigestMode>('off');
  const [settingsInitialized, setSettingsInitialized] = useState(false);

  // Seed the settings form from the group the first time it resolves.
  if (group && !settingsInitialized) {
    setEditName(group.name);
    setEditDescription(group.description ?? '');
    setEditPeerSharing(group.peer_data_sharing);
    setEditRespondMode(group.respond_mode ?? 'all');
    setEditDigestMode(group.digest_mode ?? 'off');
    setSettingsInitialized(true);
  }

  const currentUserRole: GroupRole = currentMember?.role ?? 'member';
  const isOwner = currentUserRole === 'owner';
  const isAdmin = currentUserRole === 'admin' || isOwner;
  // A member sees only their own links, at most one live per group.
  const liveLink = delegationViewer === 'member' ? (delegatedConnections[0] ?? null) : null;
  // The group's attached human coach may change where the weekly digest goes,
  // and nothing else; the server refuses any other field from them.
  // The digest select shows only where the tenant's tier sends a digest at all.
  const canSetDigest = weeklyDigest && (isAdmin || isGroupCoach);

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
      const message = describeApiError(err, { t, fallbackKey: 'groups.consentFailed' });
      showError(t('app.updateFailed'), message);
    }
  };

  const handleSaveSettings = async () => {
    if (!group) return;
    const digest = canSetDigest ? { digest_mode: editDigestMode } : {};
    try {
      await updateGroup(
        isAdmin
          ? {
              name: editName.trim() || undefined,
              description: editDescription.trim() || undefined,
              peer_data_sharing: editPeerSharing,
              respond_mode: editRespondMode,
              ...digest,
            }
          : digest,
      );
      showSuccess(t('app.settingsSaved'), t('app.groupSettingsUpdated'));
    } catch (err) {
      const message = describeApiError(err, { t, fallbackKey: 'groups.saveFailed' });
      showError(t('app.saveFailed'), message);
    }
  };

  const handleRemoveCoach = async () => {
    try {
      await removeCoach();
      showSuccess(t('humanCoach.removed'), t('humanCoach.detached'));
      setConfirmRemoveCoach(false);
    } catch (err) {
      const message = describeApiError(err, { t, fallbackKey: 'humanCoach.removeFailed' });
      showError(t('app.removeFailed'), message);
    }
  };

  const handleLeave = async () => {
    try {
      await leaveGroup(groupId);
      showSuccess(t('app.leftGroup'), t('app.youLeftGroup'));
      onMembershipEnded();
    } catch (err) {
      const message = describeApiError(err, { t, fallbackKey: 'groups.leaveFailed' });
      showError(t('app.leaveFailed'), message);
    }
  };

  const handleDelete = async () => {
    try {
      await deleteGroup(groupId);
      showSuccess(t('app.groupDeleted'), t('app.groupArchivedPermanently'));
      onMembershipEnded();
    } catch (err) {
      const message = describeApiError(err, { t, fallbackKey: 'groups.deleteFailed' });
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
            {t('groups.memberCount', { n: members.length })}
          </span>
          {isOwner && (
            <span className="flex items-center gap-1.5 text-warning">
              <Crown className="w-3.5 h-3.5" aria-hidden="true" />
              {t('groups.owner')}
            </span>
          )}
          {group.coach_user_id && (
            <span className="flex items-center gap-1.5 text-primary" data-testid="group-info-coach-badge">
              <UserCog className="w-3.5 h-3.5" aria-hidden="true" />
              {isGroupCoach ? t('humanCoach.youCoach') : t('humanCoach.attachedBadge')}
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

      {/* Which side of the links the caller is on is the server's answer:
          a coach who is also a member is still the group's coach. */}
      {delegationViewer === 'coach' ? (
        <Section icon={<Activity className="w-3.5 h-3.5" aria-hidden="true" />} title={t('delegation.sectionTitle')}>
          <DelegatedConnectionsSection
            groupId={groupId}
            mode="coach"
            connections={delegatedConnections}
            members={members}
            onOpenConnections={onOpenConnections}
          />
        </Section>
      ) : delegationViewer === 'member' && liveLink ? (
        <Section icon={<Activity className="w-3.5 h-3.5" aria-hidden="true" />} title={t('delegation.memberSectionTitle')}>
          <DelegatedConnectionsSection
            groupId={groupId}
            mode="member"
            connections={[liveLink]}
            members={members}
          />
        </Section>
      ) : null}

      {/* Only an owner or admin may list a group's invites; the route refuses anyone else. */}
      {isAdmin && (
        <Section icon={<Link2 className="w-3.5 h-3.5" aria-hidden="true" />} title={t('groups.tabInvites')}>
          <InviteManager groupId={groupId} currentUserRole={currentUserRole} />
        </Section>
      )}

      {isAdmin && (
        <Section icon={<UserCog className="w-3.5 h-3.5" aria-hidden="true" />} title={t('humanCoach.coach')}>
          <p className="text-sm text-on-surface">
            {t('humanCoach.roomHint')}
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
                {t('humanCoach.remove')}
              </Button>
            </div>
          ) : (
            <p className="text-sm text-outline">
              {t('humanCoach.noneHint')}
            </p>
          )}
        </Section>
      )}

      {(isAdmin || canSetDigest) && (
        <Section icon={<Settings className="w-3.5 h-3.5" aria-hidden="true" />} title={t('groups.tabSettings')}>
          <div className="space-y-4">
            {isAdmin && (
              <>
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
                  description={t('groups.peerSharingDescription')}
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
                  helpText={t('groups.respondMentionedHint')}
                />
              </>
            )}
            {canSetDigest && (
              <Select
                id="group-digest-mode"
                label={t('groups.digestMode')}
                value={editDigestMode}
                onChange={(e) => setEditDigestMode(e.target.value as GroupDigestMode)}
                options={(Object.keys(DIGEST_MODES) as GroupDigestMode[]).map((mode) => ({
                  value: mode,
                  label: t(DIGEST_MODES[mode].labelKey),
                }))}
                helpText={t(DIGEST_MODES[editDigestMode].hintKey)}
                data-testid="group-digest-mode"
              />
            )}
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

      {!isCoachViewer && (
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
              <p className="text-xs text-outline mt-1">{t('groups.ofTotal', { n: stats.total_members })}</p>
            </div>
            <div className="stat-card-dark">
              <p className="text-xs font-medium text-on-surface-variant mb-1">{t('groups.avgWeeklyVolume')}</p>
              <p className="text-xl font-bold text-on-surface">
                {oneDecimal(language, stats.avg_weekly_volume_km)}
                <span className="text-sm text-on-surface-variant ml-1">km</span>
              </p>
            </div>
            <div className="stat-card-dark">
              <p className="text-xs font-medium text-on-surface-variant mb-1">{t('groups.avgCtl')}</p>
              <p className="text-xl font-bold text-on-surface">
                {stats.avg_ctl !== null ? oneDecimal(language, stats.avg_ctl) : '--'}
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
      )}

      <Section icon={<MessageCircle className="w-3.5 h-3.5" aria-hidden="true" />} title={t('groups.tabRoom')}>
        <GroupTranscriptPanel groupId={groupId} />
      </Section>

      {!isCoachViewer && (
      <section className="space-y-3 rounded-lg border border-error/20 p-4">
        <h4 className="text-xs font-semibold text-error">{t('chat.dangerZone')}</h4>
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
      )}

      <ConfirmDialog
        isOpen={confirmLeave}
        onClose={() => setConfirmLeave(false)}
        onConfirm={() => void handleLeave()}
        title={t('groups.leaveGroup')}
        message={
          liveLink
            ? `${t('app.confirmLeaveGroupWeb', { group: group.name })} ${t('delegation.leaveEndsLink')}`
            : t('app.confirmLeaveGroupWeb', { group: group.name })
        }
        confirmLabel={t('app.leaveGroup')}
        variant="warning"
        isLoading={isLeaving}
      />

      <ConfirmDialog
        isOpen={confirmDelete}
        onClose={() => setConfirmDelete(false)}
        onConfirm={() => void handleDelete()}
        title={t('groups.deleteGroup')}
        message={t('app.confirmArchiveGroupWeb', { group: group.name })}
        confirmLabel={t('app.deleteGroup')}
        variant="danger"
        isLoading={isDeleting}
      />

      <ConfirmDialog
        isOpen={confirmRemoveCoach}
        onClose={() => setConfirmRemoveCoach(false)}
        onConfirm={() => void handleRemoveCoach()}
        title={t('humanCoach.remove')}
        message={t('humanCoach.detachQ')}
        confirmLabel={t('humanCoach.remove')}
        variant="warning"
        isLoading={isRemovingCoach}
      />
    </div>
  );
}
