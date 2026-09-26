// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Group info for a group thread — members, TrainingPeaks links, invites, coach, settings, analytics, room, exits
// ABOUTME: Everything the retired Groups tab held, re-homed where Telegram puts it: behind the chat header

import React, { useCallback, useMemo, useState } from 'react';
import { View, Text, TouchableOpacity, ActivityIndicator, Alert, ScrollView, Share, Switch, type ViewStyle } from 'react-native';
import { Feather } from '@expo/vector-icons';
import { useRouter } from 'expo-router';
import { MENTION_PREFIX, oneDecimal } from '@pierre/shared-constants';
import { useThemeColors } from '../../constants/theme';
import { Button, CollapsibleSection, Input, Row } from '../../components/ui';
import { useAuth } from '../../contexts/AuthContext';
import { useCoachInfo } from '../../hooks/useCoachInfo';
import {
  useCreateInvite,
  useDeactivateInvite,
  useDelegatedConnections,
  useDeleteGroup,
  useGroup,
  useGroupInvites,
  useGroupMembers,
  useGroupPermissions,
  useGroupStats,
  useLeaveGroup,
  useRemoveCoach,
  useRemoveMember,
  useUpdateGroup,
  useUpdateMemberRole,
  useUpdatePeerConsent,
} from '../../hooks/useGroups';
import { GroupInsightsSection } from './GroupInsightsSection';
import { GroupTranscriptSection } from './GroupTranscriptSection';
import { MemberRow } from './MemberRow';
import { DelegatedConnectionsSection } from './DelegatedConnectionsSection';
import { CONNECTIONS_ROUTE } from '../../navigation/routes';
import type { GroupDigestMode, GroupMember, GroupRole, UpdateGroupRequest } from '../../types';
import { useTranslation } from '@pierre/i18n';
import { describeApiError } from '@pierre/ui-logic';

/** How long an invite created from this sheet stays redeemable. */
const INVITE_LIFETIME_DAYS = 7;

/** Where a shared invite code sends someone; the web app re-homes it into chat. */
const INVITE_LINK_BASE = 'https://app.dravr.ai/groups/join';

/** Each weekly-digest mode, in the order the sheet offers them, with its label and one-line hint. */
const DIGEST_MODES: ReadonlyArray<{ mode: GroupDigestMode; labelKey: string; hintKey: string }> = [
  { mode: 'off', labelKey: 'groups.digestOff', hintKey: 'groups.digestOffHint' },
  { mode: 'chat', labelKey: 'groups.digestChat', hintKey: 'groups.digestChatHint' },
  { mode: 'managers', labelKey: 'groups.digestManagers', hintKey: 'groups.digestManagersHint' },
];

/**
 * `Row` pays its own 16px side inset for a full-bleed pane. This sheet is
 * not full-bleed: `ConversationInfoSheet` (a sibling lane's file) mounts it
 * inside a non-`flush` `ui/Sheet`, which already pays that inset for the
 * whole panel — so a bare `Row` here would stack to 32px. Wrapping a `Row`
 * in this cancels the panel's inset locally, the same net effect `<Sheet
 * flush>` gets other callers (`ChatScreen`, `ConnectionsScreen`) by passing
 * a prop this lane's files cannot touch.
 */
const CANCEL_PANEL_INSET: ViewStyle = { marginHorizontal: -16 };

export interface GroupInfoSheetProps {
  /** The group this thread is scoped to. */
  groupId: string;
  /** The thread's own title, shown until the group loads. */
  fallbackName: string | null;
  /** Close the host sheet. */
  onClose: () => void;
  /** The athlete is no longer in this group: go back to the conversation list. */
  onLeft: () => void;
}

/**
 * Everything a member or an admin can do about the group, from inside the
 * group's own chat.
 *
 * Sections collapse because a sheet over a transcript has little room and an
 * athlete usually opens it for one thing. The admin controls appear only for
 * an admin: the API refuses them from anyone else, so showing them would
 * advertise a 403.
 */
export function GroupInfoSheet({ groupId, fallbackName, onClose, onLeft }: GroupInfoSheetProps) {
  const { t, language } = useTranslation();
  const colors = useThemeColors();
  const { user } = useAuth();
  const router = useRouter();

  const { group, isLoading: isLoadingGroup } = useGroup(groupId);
  const { members, isLoading: isLoadingMembers } = useGroupMembers(groupId);

  // The caller's own membership row, matched on their user id. Taking the
  // first active row instead handed whoever the server listed first the
  // caller's role badge and, worse, the caller's consent switch.
  const myMembership = useMemo(
    () => members.find((member) => member.user_id === user?.id),
    [members, user?.id],
  );
  // The group's human coach holds no membership row: they read the group and
  // its members, link TrainingPeaks athletes, and follow the room, while the
  // member-only surfaces (consent, invites, settings, analytics, the exits)
  // stay off, since their routes refuse a non-member.
  const isGroupCoach = !!group?.coach_user_id && group.coach_user_id === user?.id;
  const isCoachViewer = isGroupCoach && !myMembership;
  // Held until the group resolves: before then a coach reads as a member.
  const memberSurfaces = !!group && !isCoachViewer;

  const { stats, isLoading: isLoadingStats } = useGroupStats(groupId, memberSurfaces);
  const isAdmin = myMembership?.role === 'owner' || myMembership?.role === 'admin';
  // Only an owner or admin may list a group's invites; the route refuses anyone else.
  const { invites, isLoading: isLoadingInvites } = useGroupInvites(groupId, memberSurfaces && isAdmin);
  const { connections: delegatedConnections, viewer: delegationViewer } = useDelegatedConnections(
    groupId,
    !!group?.coach_user_id,
  );
  // A member sees only their own links, at most one live per group.
  const liveLink = delegationViewer === 'member' ? (delegatedConnections[0] ?? null) : null;
  const { weeklyDigest } = useGroupPermissions();
  const { createInvite, isPending: isCreatingInvite } = useCreateInvite(groupId);
  const { deactivateInvite } = useDeactivateInvite(groupId);
  const { updateGroup, isPending: isUpdatingGroup } = useUpdateGroup(groupId);
  const { updateConsent, isPending: isSavingConsent } = useUpdatePeerConsent(groupId);
  const { updateRole } = useUpdateMemberRole(groupId);
  const { removeMember } = useRemoveMember(groupId);
  const { removeCoach } = useRemoveCoach(groupId);
  const { leaveGroup, isPending: isLeaving } = useLeaveGroup();
  const { deleteGroup, isPending: isDeleting } = useDeleteGroup();
  const { coach: aiCoach } = useCoachInfo(group?.agent_id ?? null);

  const [removingMemberId, setRemovingMemberId] = useState<string | null>(null);
  const [roleChangingUserId, setRoleChangingUserId] = useState<string | null>(null);
  const [nameDraft, setNameDraft] = useState<string | null>(null);
  const [descriptionDraft, setDescriptionDraft] = useState<string | null>(null);

  const isOwner = myMembership?.role === 'owner';
  // The group's attached human coach may change where the weekly digest goes,
  // and nothing else; the digest rows show only where the tier sends one.
  const canSetDigest = weeklyDigest && (isAdmin || isGroupCoach);
  const activeInvites = useMemo(() => invites.filter((invite) => invite.is_active), [invites]);

  const handleRemoveMember = useCallback(
    (member: GroupMember) => {
      Alert.alert(t('app.removeMember'), t('app.confirmRemoveMember', { member: member.display_name ?? t('app.thisMember') }), [
        { text: t('common.cancel'), style: 'cancel' },
        {
          text: t('app.remove'),
          style: 'destructive',
          onPress: async () => {
            try {
              setRemovingMemberId(member.user_id);
              await removeMember(member.user_id);
            } catch (err) {
              Alert.alert(t('common.error'), describeApiError(err, { t, fallbackKey: 'app.failedRemoveMember' }));
            } finally {
              setRemovingMemberId(null);
            }
          },
        },
      ]);
    },
    [removeMember, t],
  );

  const handleChangeRole = useCallback(
    async (member: GroupMember, role: GroupRole) => {
      try {
        setRoleChangingUserId(member.user_id);
        await updateRole({ userId: member.user_id, role });
      } catch (err) {
        Alert.alert(t('common.error'), describeApiError(err, { t, fallbackKey: 'app.failedUpdateRole' }));
      } finally {
        setRoleChangingUserId(null);
      }
    },
    [updateRole, t],
  );

  const createInviteOfKind = useCallback(
    async (kind: 'member' | 'coach') => {
      try {
        const invite = await createInvite(
          kind === 'coach'
            ? { expires_in_days: INVITE_LIFETIME_DAYS, kind: 'coach' as const }
            : { expires_in_days: INVITE_LIFETIME_DAYS },
        );
        await Share.share({
          message:
            kind === 'coach'
              ? `Coach invite for ${group?.name ?? 'our group'}: ${INVITE_LINK_BASE}/${invite.code}`
              : `Join ${group?.name ?? 'our group'}: ${INVITE_LINK_BASE}/${invite.code}`,
        });
      } catch (err) {
        Alert.alert(t('common.error'), describeApiError(err, { t, fallbackKey: 'app.failedCreateInvite' }));
      }
    },
    [createInvite, group?.name, t],
  );

  const handleShareInvite = useCallback(() => {
    Alert.alert(t('app.createInvite'), t('app.whoIsInviteFor'), [
      { text: t('app.inviteKindMember'), onPress: () => void createInviteOfKind('member') },
      { text: t('humanCoach.coach'), onPress: () => void createInviteOfKind('coach') },
      { text: t('common.cancel'), style: 'cancel' },
    ]);
  }, [createInviteOfKind, t]);

  const handleDeactivateInvite = useCallback(
    (inviteId: string, code: string) => {
      Alert.alert(t('app.deactivateInvite'), t('app.confirmDeactivateInvite', { code }), [
        { text: t('common.cancel'), style: 'cancel' },
        {
          text: t('app.deactivate'),
          style: 'destructive',
          onPress: async () => {
            try {
              await deactivateInvite(inviteId);
            } catch (err) {
              Alert.alert(t('common.error'), describeApiError(err, { t, fallbackKey: 'app.failedDeactivateInvite' }));
            }
          },
        },
      ]);
    },
    [deactivateInvite, t],
  );

  const handleRemoveCoach = useCallback(() => {
    Alert.alert(t('humanCoach.remove'), t('humanCoach.detachQ'), [
      { text: t('common.cancel'), style: 'cancel' },
      {
        text: t('app.remove'),
        style: 'destructive',
        onPress: async () => {
          try {
            await removeCoach();
          } catch (err) {
            Alert.alert(t('common.error'), describeApiError(err, { t, fallbackKey: 'humanCoach.removeFailed' }));
          }
        },
      },
    ]);
  }, [removeCoach, t]);

  const saveIdentity = useCallback(async () => {
    const name = (nameDraft ?? group?.name ?? '').trim();
    if (!name) return;
    try {
      await updateGroup({ name, description: (descriptionDraft ?? group?.description ?? '').trim() });
      setNameDraft(null);
      setDescriptionDraft(null);
    } catch (err) {
      Alert.alert(t('common.error'), describeApiError(err, { t, fallbackKey: 'app.failedUpdateGroup' }));
    }
  }, [nameDraft, descriptionDraft, group?.name, group?.description, updateGroup, t]);

  const setGroupFlag = useCallback(
    async (patch: Pick<UpdateGroupRequest, 'peer_data_sharing' | 'respond_mode' | 'digest_mode'>) => {
      try {
        await updateGroup(patch);
      } catch (err) {
        Alert.alert(t('common.error'), describeApiError(err, { t, fallbackKey: 'app.failedUpdateGroup' }));
      }
    },
    [updateGroup, t],
  );

  const handleConsentChange = useCallback(
    async (consent: boolean) => {
      try {
        await updateConsent(consent);
      } catch (err) {
        Alert.alert(t('common.error'), describeApiError(err, { t, fallbackKey: 'app.failedUpdateSharing' }));
      }
    },
    [updateConsent, t],
  );

  const openConnections = useCallback(() => {
    onClose();
    router.push(CONNECTIONS_ROUTE as never);
  }, [onClose, router]);

  const handleLeave = useCallback(() => {
    const question = t('app.confirmLeaveGroup', { group: group?.name ?? t('app.thisGroup') });
    Alert.alert(t('app.leaveGroup'), liveLink ? `${question} ${t('delegation.leaveEndsLink')}` : question, [
      { text: t('common.cancel'), style: 'cancel' },
      {
        text: t('app.leave'),
        style: 'destructive',
        onPress: async () => {
          try {
            await leaveGroup(groupId);
            onClose();
            onLeft();
          } catch (err) {
            Alert.alert(t('common.error'), describeApiError(err, { t, fallbackKey: 'app.failedLeaveGroup' }));
          }
        },
      },
    ]);
  }, [group?.name, groupId, leaveGroup, liveLink, onClose, onLeft, t]);

  const handleDelete = useCallback(() => {
    Alert.alert(
      t('app.archiveGroupTitle'),
      t('app.confirmArchiveGroup', { group: group?.name ?? t('app.thisGroup') }),
      [
        { text: t('common.cancel'), style: 'cancel' },
        {
          text: t('app.archive'),
          style: 'destructive',
          onPress: async () => {
            try {
              await deleteGroup(groupId);
              onClose();
              onLeft();
            } catch (err) {
              Alert.alert(t('common.error'), describeApiError(err, { t, fallbackKey: 'app.failedArchiveGroup' }));
            }
          },
        },
      ],
    );
  }, [group?.name, groupId, deleteGroup, onClose, onLeft, t]);

  if (isLoadingGroup && !group) {
    return (
      <View className="py-10 items-center" testID="group-info-loading">
        <ActivityIndicator size="large" color={colors.pierre.violet} />
      </View>
    );
  }

  return (
    <ScrollView testID="group-info-sheet" keyboardShouldPersistTaps="handled">
      <Text className="text-lg font-bold text-text-primary" testID="group-info-name">
        {group?.name ?? fallbackName ?? t('app.group')}
      </Text>
      {group?.description ? (
        <Text className="text-sm text-text-secondary mt-1" testID="group-info-description">
          {group.description}
        </Text>
      ) : null}

      <View className="mt-4">
        <CollapsibleSection
          title={t('app.membersCount', { count: members.length })}
          defaultExpanded
          testID="group-info-members"
        >
          {isLoadingMembers ? (
            <ActivityIndicator size="small" color={colors.pierre.violet} />
          ) : (
            members.map((member, index) => (
              <MemberRow
                key={member.id}
                member={member}
                isAdmin={isAdmin}
                isOwner={isOwner}
                onRemove={handleRemoveMember}
                onChangeRole={handleChangeRole}
                isRemoving={removingMemberId === member.user_id}
                isChangingRole={roleChangingUserId === member.user_id}
                last={index === members.length - 1}
              />
            ))
          )}
        </CollapsibleSection>

        {/* Which side of the links the caller is on is the server's answer:
            a coach who is also a member is still the group's coach. */}
        {delegationViewer === 'coach' ? (
          <CollapsibleSection title={t('delegation.sectionTitle')} defaultExpanded testID="group-info-delegation">
            <DelegatedConnectionsSection
              groupId={groupId}
              mode="coach"
              connections={delegatedConnections}
              members={members}
              onOpenConnections={openConnections}
            />
          </CollapsibleSection>
        ) : liveLink ? (
          <CollapsibleSection title={t('delegation.memberSectionTitle')} defaultExpanded testID="group-info-delegation">
            <DelegatedConnectionsSection
              groupId={groupId}
              mode="member"
              connections={[liveLink]}
              members={members}
            />
          </CollapsibleSection>
        ) : null}

        {isAdmin && (
          <CollapsibleSection title={`Invites (${activeInvites.length})`} testID="group-info-invites">
            <View style={CANCEL_PANEL_INSET}>
              <Row
                title={t('app.createShareInvite')}
                onPress={handleShareInvite}
                trailing={isCreatingInvite ? <ActivityIndicator size="small" color={colors.pierre.violet} /> : undefined}
                last={false}
                testID="group-info-create-invite"
              />
            </View>

            {isLoadingInvites ? (
              <View className="px-4">
                <ActivityIndicator size="small" color={colors.pierre.violet} />
              </View>
            ) : activeInvites.length === 0 ? (
              <Text className="text-sm text-text-tertiary py-2 px-4" testID="group-invites-empty">
                {t('app.noActiveInvites')}
              </Text>
            ) : (
              <View style={CANCEL_PANEL_INSET}>
                {activeInvites.map((invite, index) => (
                  <Row
                    key={invite.id}
                    title={invite.code}
                    subtitle={`${invite.kind === 'coach' ? t('humanCoach.invite') : t('app.memberInvite')} · used ${invite.use_count}×`}
                    trailing={
                      <Button
                        title={t('app.deactivate')}
                        variant="danger"
                        onPress={() => handleDeactivateInvite(invite.id, invite.code)}
                        testID={`deactivate-invite-${invite.id}`}
                      />
                    }
                    last={index === activeInvites.length - 1}
                    testID={`group-invite-${invite.id}`}
                  />
                ))}
              </View>
            )}
          </CollapsibleSection>
        )}

        <CollapsibleSection title={t('humanCoach.coach')} testID="group-info-coach">
          <View className="flex-row items-center py-2">
            <Feather name="cpu" size={16} color={colors.pierre.violet} />
            <Text className="text-sm text-text-primary ml-2 flex-1" testID="group-info-ai-coach">
              {aiCoach?.title ?? t('app.aiAgent')}
              {aiCoach?.handle ? ` · ${MENTION_PREFIX}${aiCoach.handle}` : ''}
            </Text>
          </View>
          {group?.coach_user_id ? (
            <View className="flex-row items-center py-2" testID="group-info-human-coach">
              <Feather name="user-check" size={16} color={colors.pierre.violet} />
              <Text className="text-sm text-text-primary ml-2 flex-1">
                {isGroupCoach ? t('humanCoach.youCoach') : t('humanCoach.attached')}
              </Text>
              {isAdmin && (
                <TouchableOpacity onPress={handleRemoveCoach} testID="remove-coach-button">
                  <Text className="text-sm font-semibold text-text-secondary">{t('app.remove')}</Text>
                </TouchableOpacity>
              )}
            </View>
          ) : (
            <Text className="text-xs text-text-tertiary py-2">
              {t('humanCoach.none')}
            </Text>
          )}
        </CollapsibleSection>

        {/* A coach viewer reaches Settings only for the digest rows; the admin
            rows and the consent card inside stay bound to a membership row. */}
        {(!isCoachViewer || canSetDigest) && (
        <CollapsibleSection title={t('common.settings')} testID="group-info-settings">
          {isAdmin && group && (
            <>
              <Input
                label={t('app.name')}
                value={nameDraft ?? group.name}
                onChangeText={setNameDraft}
                testID="group-name-input"
              />
              <Input
                label={t('app.description')}
                value={descriptionDraft ?? group.description ?? ''}
                onChangeText={setDescriptionDraft}
                testID="group-description-input"
              />
              <Button
                title={t('common.save')}
                onPress={() => void saveIdentity()}
                disabled={isUpdatingGroup || (nameDraft === null && descriptionDraft === null)}
                fullWidth
                style={{ marginBottom: 12 }}
                testID="group-save-identity"
              />

              <View style={CANCEL_PANEL_INSET}>
                <Row
                  title={t('app.peerDataSharing')}
                  subtitle={t('app.peerCompareBlurb')}
                  trailing={
                    <Switch
                      value={group.peer_data_sharing}
                      onValueChange={(value) => void setGroupFlag({ peer_data_sharing: value })}
                      disabled={isUpdatingGroup}
                      trackColor={{ false: colors.border.default, true: colors.pierre.violet }}
                      testID="group-peer-sharing-switch"
                    />
                  }
                  testID="group-peer-sharing-row"
                />
                <Row
                  title={t('app.replyOnMentionOnly')}
                  subtitle={t('app.mentionOnlyOffNote')}
                  trailing={
                    <Switch
                      value={group.respond_mode === 'mentions'}
                      onValueChange={(value) => void setGroupFlag({ respond_mode: value ? 'mentions' : 'all' })}
                      disabled={isUpdatingGroup}
                      trackColor={{ false: colors.border.default, true: colors.pierre.violet }}
                      testID="group-respond-mode-switch"
                    />
                  }
                  last={!myMembership && !canSetDigest}
                  testID="group-respond-mode-row"
                />
              </View>
            </>
          )}

          {/* Where the weekly digest goes: a radio list, the check on the mode
              in force. The coach reaches this block without the admin rows. */}
          {canSetDigest && group && (
            <View style={CANCEL_PANEL_INSET} testID="group-digest-mode">
              <Text className="px-4 pt-3 pb-1 text-xs font-semibold text-text-tertiary">
                {t('groups.digestMode')}
              </Text>
              {DIGEST_MODES.map(({ mode, labelKey, hintKey }, index) => {
                const isSelected = group.digest_mode === mode;
                return (
                  <Row
                    key={mode}
                    title={t(labelKey)}
                    subtitle={t(hintKey)}
                    onPress={() => {
                      if (!isSelected && !isUpdatingGroup) void setGroupFlag({ digest_mode: mode });
                    }}
                    showChevron={false}
                    last={!myMembership && index === DIGEST_MODES.length - 1}
                    accessibilityRole="radio"
                    accessibilityState={{ selected: isSelected }}
                    accessibilityLabel={t(labelKey)}
                    trailing={isSelected ? <Feather name="check" size={18} color={colors.tokens.primary} /> : undefined}
                    testID={`group-digest-mode-${mode}`}
                  />
                );
              })}
            </View>
          )}

          {/* The caller's own consent. The group can allow peer sharing, but
              each athlete still decides whether their own training data is
              part of it, and this is the only in-app place to say so. */}
          {myMembership && (
            <View style={CANCEL_PANEL_INSET} testID="peer-consent-card">
              <Row
                title={t('app.shareMyTrainingData')}
                subtitle={group?.peer_data_sharing ? t('app.shareTrainingBlurb') : t('app.groupSharingOffNote')}
                trailing={
                  <Switch
                    value={myMembership.peer_sharing_consent}
                    onValueChange={(value) => void handleConsentChange(value)}
                    disabled={isSavingConsent}
                    trackColor={{ false: colors.border.default, true: colors.pierre.violet }}
                    testID="peer-consent-switch"
                  />
                }
                last
                testID="peer-consent-row"
              />
            </View>
          )}
        </CollapsibleSection>
        )}

        {!isCoachViewer && (
        <CollapsibleSection title={t('app.analytics')} testID="group-info-analytics">
          <View className="flex-row py-2">
            <View className="flex-1 items-center">
              <Text className="text-lg font-bold font-mono tabular-nums text-text-primary" testID="group-stat-members">
                {members.length}
              </Text>
              <Text className="text-xs text-text-tertiary mt-0.5">{t('app.members')}</Text>
            </View>
            <View className="flex-1 items-center">
              <Text className="text-lg font-bold font-mono tabular-nums text-text-primary" testID="group-stat-active">
                {isLoadingStats ? '…' : String(stats?.active_members ?? members.length)}
              </Text>
              <Text className="text-xs text-text-tertiary mt-0.5">{t('app.active')}</Text>
            </View>
            <View className="flex-1 items-center">
              <Text className="text-lg font-bold font-mono tabular-nums text-text-primary" testID="group-stat-volume">
                {isLoadingStats
                  ? '…'
                  : stats?.avg_weekly_volume_km !== undefined
                    ? oneDecimal(language, stats.avg_weekly_volume_km)
                    : '--'}
              </Text>
              <Text className="text-xs text-text-tertiary mt-0.5">{t('groups.avgVolumeKm')}</Text>
            </View>
          </View>
          <GroupInsightsSection groupId={groupId} isAdmin={isAdmin} weeklyDigestEnabled={weeklyDigest} />
        </CollapsibleSection>
        )}

        <CollapsibleSection title={t('app.room')} testID="group-info-room">
          <GroupTranscriptSection groupId={groupId} />
        </CollapsibleSection>
      </View>

      {!isOwner && !isCoachViewer && (
        <Button
          title={t('app.leaveGroupLower')}
          onPress={handleLeave}
          loading={isLeaving}
          variant="danger"
          fullWidth
          style={{ marginTop: 8 }}
          testID="leave-group-button"
        />
      )}

      {isOwner && (
        <Button
          title={t('app.archiveGroup')}
          onPress={handleDelete}
          loading={isDeleting}
          variant="danger"
          fullWidth
          style={{ marginTop: 8 }}
          testID="archive-group-button"
        />
      )}
    </ScrollView>
  );
}
