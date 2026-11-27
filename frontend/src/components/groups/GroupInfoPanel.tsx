// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Group info for the open group thread, ordered by who reads it — the owner waiting on their seat, the coach, an athlete
// ABOUTME: What each viewer needs first leads; settings, the week and the room history fold into rows; leaving is a quiet last row

import { useState } from 'react';
import { clsx } from 'clsx';
import {
  useCoachConsents,
  useCoachSharing,
  useDelegatedConnections,
  useDelegationRoster,
  useGroup,
  useGroupMembers,
  useGroupPermissions,
  useGroupStats,
  useUpdatePeerConsent,
  useSetCoachSharing,
  useSetCoachConsents,
  useLeaveGroup,
  useDeleteGroup,
  useRemoveCoach,
} from '../../hooks/useGroups';
import { useAuth } from '../../hooks/useAuth';
import { useCoachSeat } from '../../hooks/useCoachSeat';
import { useOpenAthleteThread } from '../../hooks/useCoachPlan';
import {
  Checkbox,
  ConfirmDialog,
  EmptyState,
  OverflowMenu,
  Section,
  useErrorToast,
  useSuccessToast,
} from '../ui';
import MemberList from './MemberList';
import AthletePlanModal from './AthletePlanModal';
import AthletePhysiologyModal from './AthletePhysiologyModal';
import { AgentRow, CoachRow } from './GroupLeads';
import CoachAccessRequest from '../CoachAccessRequest';
import ClaimCoachBlock from './ClaimCoachBlock';
import InviteManager from './InviteManager';
import GroupInsightsPanel from './GroupInsightsPanel';
import GroupTranscriptPanel from './GroupTranscriptPanel';
import DelegatedConnectionsSection from './DelegatedConnectionsSection';
import CoachAssistantNotice from './CoachAssistantNotice';
import RoutineDetectionNotice from './RoutineDetectionNotice';
import CoachConsentsSection from './CoachConsentsSection';
import CoachReadLog from './CoachReadLog';
import CoachAthleteCard from './CoachAthleteCard';
import GroupSettingsForm from './GroupSettingsForm';
import GroupChatConnection from './GroupChatConnection';
import WhoSeesWhat from './WhoSeesWhat';
import { useMemberActions } from './useMemberActions';
import { DisclosureRow, QuietExitRow, ROW_DIVIDER, RowBox, ValueRow } from './GroupInfoParts';
import { RESPOND_MODE_LABEL_KEY } from './groupSettingsLabels';
import type { GroupMember, GroupRole, GroupTrend } from '@pierre/shared-types';
import {
  COACH_CONSENTS_KEYS,
  DIGEST_MODES,
  analyticsScopeLine,
  coachPlatformName,
  garminContributionLine,
  offeredConsents,
  oneDecimal,
  weeklyVolumeFigure,
} from '@pierre/shared-constants';
import { avatarSlot, distanceSymbol, initialsFor } from '@pierre/chat-utils';
import { useTranslation } from '@pierre/i18n';
import { describeApiError, openThreadRefusalText, useDistanceUnit } from '@pierre/ui-logic';
import { avatarShapeClass, avatarSlotClass } from '../chat/avatarSlots';

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
  /**
   * Open a conversation by id — where the coach lands on their plan thread
   * for one member. Without it, the roster offers no way into one.
   */
  onOpenConversation?: (conversationId: string) => void;
  /** Open one athlete on the coach's Home; offered only to a coach who has one. */
  onOpenCoachHome?: (athleteId: string) => void;
}

// Built at import time, where `t` does not exist: the table carries the key
// and the render resolves it.
const TREND_DISPLAY: Record<GroupTrend, { labelKey: string; color: string }> = {
  improving: { labelKey: 'groups.improving', color: 'text-on-success-container' },
  stable: { labelKey: 'groups.stable', color: 'text-on-surface-variant' },
  declining: { labelKey: 'groups.declining', color: 'text-on-warning-container' },
};

/**
 * Below this many members sharing with the reader, the group's averages
 * describe one or two people — an average CTL of a pilot's one athlete is
 * just their CTL — so the figures stay folded away unshown.
 */
const STATS_FROM_SHARERS = 3;

/** Who is reading the drawer, which decides what leads it. */
type Viewer = 'seat' | 'coach' | 'member';

/**
 * Everything about the group behind the open thread, in the order its reader
 * needs it (canvas row 3, P2·A):
 *
 * - **The owner offered the coach seat** meets the seat block first — what is
 *   blocked and the drawer's one filled button — then who is here, with the
 *   invite link inline while there is nobody to coach yet.
 * - **The group's coach** meets their athlete first, with the plan thread as
 *   coach Home weighs it, then the planning connections, then the agent.
 * - **An athlete** reads who is here and what each of them sees, then their
 *   own sharing, then what their coach may change.
 *
 * Settings (saved as they change), the week and the room history fold into
 * rows for everyone; leaving or deleting is a quiet row at the very bottom,
 * and the danger is drawn only inside the confirm it opens.
 */
export default function GroupInfoPanel({
  groupId,
  onMembershipEnded,
  onOpenConnections,
  onOpenConversation,
  onOpenCoachHome,
}: GroupInfoPanelProps) {
  const { t, language } = useTranslation();
  const units = useDistanceUnit();
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
  // The coach's name as the server spells it: display name, else email.
  const coachName = group?.coach_user_id ? group.coach_display_name : null;

  const { stats, scope: statsScope, isLoading: isStatsLoading } = useGroupStats(
    groupId,
    // Held until the group resolves: before then a coach reads as a member.
    !!group && !isCoachViewer,
  );
  const statsScopeLine = analyticsScopeLine(t, statsScope);
  // The stats, the weekly report and the health flags are computed over the
  // same members, so one line beneath the section's title attributes them all
  // when Garmin contributed to their sessions (carnet#1154).
  const garminLine = garminContributionLine(t, statsScope?.garmin_contribution);
  const { connections: delegatedConnections, viewer: delegationViewer } = useDelegatedConnections(
    groupId,
    !!group?.coach_user_id,
  );
  // What the member lets the group's coach do lives on their membership, so a
  // member of a coached group reads it whether or not a link names them.
  const { consents: coachConsents } = useCoachConsents(
    groupId,
    !!group?.coach_user_id && !!currentMember && !isGroupCoach,
  );
  const { weeklyDigest } = useGroupPermissions();
  const { updateConsent, isPending: isSavingConsent } = useUpdatePeerConsent(groupId);
  // The owner offered this group's coach seat (carnet#1520): the seat block
  // leads the drawer, and nothing below tells them someone else will coach.
  const seat = useCoachSeat(groupId);
  // Whether the group's human coach sees the member's training: granted at
  // join, so the member's own switch leads their sharing (carnet#1168).
  // The owner about to be the coach is never asked to share with a coach.
  const showsCoachSharing = !!currentMember && !isGroupCoach && !seat.offered;
  const { sharing: coachSharing } = useCoachSharing(groupId, showsCoachSharing);
  const { setSharing: setCoachSharing, isPending: isSavingCoachSharing } =
    useSetCoachSharing(groupId);
  // Whether the coach's AI assistant includes the member in its roster tools
  // (carnet#1511), off until the member turns it on here, whether the coach
  // is told of missed planned sessions (carnet#1792), on until the member
  // turns it off here, and whether the coach is told of a break unusual for
  // the member (carnet#1792), off until the member turns it on here: all per
  // membership.
  const { setConsents: setCoachConsents, isPending: isSavingCoachConsents } =
    useSetCoachConsents(groupId);
  const { leaveGroup, isPending: isLeaving } = useLeaveGroup();
  const { deleteGroup, isPending: isDeleting } = useDeleteGroup();
  const { removeCoach, isPending: isRemovingCoach } = useRemoveCoach(groupId);
  const { openThread, pendingAthleteId } = useOpenAthleteThread();
  const showError = useErrorToast();
  const showSuccess = useSuccessToast();

  const [confirmLeave, setConfirmLeave] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [confirmRemoveCoach, setConfirmRemoveCoach] = useState(false);
  // The member whose live plan the coach is reading, read-only (carnet#1244).
  const [viewingPlanFor, setViewingPlanFor] = useState<GroupMember | null>(null);
  const [viewingThresholdsFor, setViewingThresholdsFor] = useState<GroupMember | null>(null);

  const currentUserRole: GroupRole = currentMember?.role ?? 'member';
  const isOwner = currentUserRole === 'owner';
  const isAdmin = currentUserRole === 'admin' || isOwner;
  // The coach who also owns or administers the group changes roles and
  // removes members from each athlete's card, as the rows do for everyone else.
  const athleteActions = useMemberActions(groupId, currentUserRole, currentUserId);
  // A member sees only their own links: one live per coaching platform the
  // group's coach links them on, each with its own controls.
  const memberLinks = delegationViewer === 'member' ? delegatedConnections : [];
  // The coaching platform the coach's links are on, which their title names:
  // the coach's roster names it (the section reads the same cached roster).
  // Each of a member's links names its own.
  const { provider: rosterProvider } = useDelegationRoster(groupId, delegationViewer === 'coach');
  const rosterPlatform = coachPlatformName(rosterProvider);
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

  /** Stop or resume sharing the caller's own training with the group's coach. */
  const handleCoachSharingChange = async (consent: boolean) => {
    try {
      await setCoachSharing(consent);
      showSuccess(
        consent ? t('groups.sharingOn') : t('groups.sharingOff'),
        consent ? t('humanCoach.sharingOnNotice') : t('humanCoach.sharingOffNotice'),
      );
    } catch (err) {
      const message = describeApiError(err, { t, fallbackKey: 'groups.consentFailed' });
      showError(t('app.updateFailed'), message);
    }
  };

  /** Let the coach's AI assistant include the caller in its roster tools, or stop it. */
  const handleAssistantReadChange = async (granted: boolean) => {
    try {
      await setCoachConsents({ assistant_read: granted });
      showSuccess(t('delegation.consentsSavedToast'));
    } catch (err) {
      const message = describeApiError(err, { t, fallbackKey: 'groups.consentFailed' });
      showError(t('app.updateFailed'), message);
    }
  };

  /** Let the coach be told when the caller's planned sessions are missed, or stop it. */
  const handleMissedSessionCheckChange = async (on: boolean) => {
    try {
      await setCoachConsents({ missed_session_check: on });
      showSuccess(t('delegation.consentsSavedToast'));
    } catch (err) {
      const message = describeApiError(err, { t, fallbackKey: 'groups.consentFailed' });
      showError(t('app.updateFailed'), message);
    }
  };

  /**
   * Let the coach be told when the caller's break is unusual for them, or
   * stop it. The switch and the one-time card both answer here; the server
   * stamps every answer, "Not now" (`false` while already off) included, so
   * its reply clears the card.
   */
  const handleRoutineDetectionChange = async (on: boolean) => {
    try {
      await setCoachConsents({ routine_detection: on });
      showSuccess(t('delegation.consentsSavedToast'));
    } catch (err) {
      const message = describeApiError(err, { t, fallbackKey: 'groups.consentFailed' });
      showError(t('app.updateFailed'), message);
    }
  };

  /**
   * Open the coach's plan thread for one member and go to it. The server
   * checks the coaching link by id; a refusal leaves the coach where they
   * are, told that the athlete stopped sharing when that is why, else asked
   * to try again.
   */
  const handleWorkOnPlan = async (member: GroupMember) => {
    if (!onOpenConversation) return;
    try {
      const thread = await openThread(member.user_id, groupId);
      setViewingPlanFor(null);
      onOpenConversation(thread.id);
    } catch (err) {
      showError(openThreadRefusalText(err, member.display_name ?? t('groups.memberFallback'), t));
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

  const viewer: Viewer = seat.offered ? 'seat' : isGroupCoach ? 'coach' : 'member';
  // Everyone in the group but the viewer: the coach's athletes, or the others an athlete shares the room with.
  const others = members.filter((m) => m.user_id !== currentUserId);
  const showsPeerConsent = !!currentMember && group.peer_data_sharing;
  const showsSharing = showsCoachSharing || showsPeerConsent;
  const statsShown = !!stats && (statsScope?.visible_members ?? 0) >= STATS_FROM_SHARERS;
  const showsThisWeek = !isCoachViewer && (isAdmin || statsShown);
  const showsSettings = isAdmin || canSetDigest;
  const roomAvatar = avatarSlot({ id: group.id, agent_id: null, group_id: group.id, subject_user_id: null });
  const canOpenCoachHome = auth.user?.manages_roster === true && !!onOpenCoachHome;

  // The member list's `⋯` holds role and removal; the coach row's holds
  // detaching the coach, for an admin of a group someone else coaches.
  const coachMenu =
    isAdmin && group.coach_user_id && !isGroupCoach ? (
      <OverflowMenu
        label={t('groups.info.moreActions', { name: coachName ?? t('humanCoach.attached') })}
        items={[
          {
            label: t('humanCoach.removeCoachRow'),
            danger: true,
            onSelect: () => setConfirmRemoveCoach(true),
            'data-testid': 'group-info-remove-coach',
          },
        ]}
        data-testid="group-info-coach-menu"
      />
    ) : undefined;

  const invites = (inline: boolean) => (
    <InviteManager
      groupId={groupId}
      currentUserRole={currentUserRole}
      offerCoachKind={!seat.offered}
      quietCreate={seat.offered}
      heading={inline && others.length === 0 ? t('groups.info.inviteAthlete') : undefined}
    />
  );

  return (
    <div className="flex flex-col gap-6" data-testid="group-info-panel">
      <header className="flex items-center gap-3">
        <span
          aria-hidden="true"
          className={clsx(
            'flex h-11 w-11 shrink-0 select-none items-center justify-center text-sm font-semibold',
            avatarShapeClass('square'),
            avatarSlotClass(roomAvatar),
          )}
        >
          {initialsFor(group.name)}
        </span>
        <div className="min-w-0">
          <h3 className="font-display text-lg font-semibold text-on-surface" data-testid="group-info-name">
            {group.name}
          </h3>
          <p className="flex flex-wrap items-center gap-x-1.5 text-xs text-on-surface-variant">
            {group.coach_user_id && (
              <>
                <span data-testid="group-info-coach-badge">
                  {isGroupCoach ? (
                    t('humanCoach.youCoach')
                  ) : coachName ? (
                    <>
                      {t('humanCoach.coach')} <span className="font-medium text-on-surface">{coachName}</span>
                    </>
                  ) : (
                    t('humanCoach.attachedBadge')
                  )}
                </span>
                <span aria-hidden="true">·</span>
              </>
            )}
            <span>{t('groups.memberCount', { n: members.length })}</span>
          </p>
        </div>
      </header>
      {group.description && (
        <p className="-mt-3 text-pretty text-sm text-on-surface-variant" data-testid="group-info-description">
          {group.description}
        </p>
      )}

      {/* The coach seat leads the drawer for the owner it is offered to: what
          is blocked, the drawer's one filled request, the coaching platforms;
          once asked, the dated timeline (carnet#1520). */}
      {viewer === 'seat' && <CoachAccessRequest groupId={groupId} variant="block" />}

      {/* An owner who already holds coach access, on a group nobody coaches:
          the one way to take the seat (carnet#1650). */}
      {viewer === 'member' && isOwner && !group.coach_user_id && auth.user?.manages_roster === true && (
        <ClaimCoachBlock groupId={groupId} />
      )}

      {viewer === 'coach' && (
        <Section
          headingLevel={3}
          title={others.length === 1 ? t('humanCoach.athleteCard.titleOne') : t('humanCoach.athleteCard.titleMany')}
          data-testid="group-info-athletes"
        >
          {isMembersLoading ? (
            <div className="flex justify-center py-6">
              <div className="pierre-spinner" />
            </div>
          ) : others.length === 0 ? (
            <EmptyState>{t('groups.info.noAthlete')}</EmptyState>
          ) : (
            <div className="space-y-3">
              {others.map((member) => (
                <CoachAthleteCard
                  key={member.id}
                  member={member}
                  primary={others.length === 1}
                  onWorkOnPlan={onOpenConversation ? (m) => void handleWorkOnPlan(m) : undefined}
                  opening={pendingAthleteId === member.user_id}
                  busy={pendingAthleteId !== null}
                  onViewPlan={setViewingPlanFor}
                  onViewThresholds={setViewingThresholdsFor}
                  onOpenCoachHome={canOpenCoachHome ? (m) => onOpenCoachHome?.(m.user_id) : undefined}
                  menu={
                    athleteActions.itemsFor(member).length > 0 ? (
                      <OverflowMenu
                        label={t('groups.info.moreActions', { name: member.display_name ?? t('groups.memberFallback') })}
                        items={athleteActions.itemsFor(member)}
                        data-testid={`member-menu-${member.user_id}`}
                      />
                    ) : undefined
                  }
                />
              ))}
            </div>
          )}
          {athleteActions.dialog}
        </Section>
      )}

      {viewer !== 'coach' && (
        <Section headingLevel={3} title={t('groups.info.whoTitle')} data-testid="group-info-people">
          <RowBox>
            <CoachRow
              group={group}
              // While the request is read, the owner is still not the coach.
              seatState={viewer === 'seat' ? (seat.state ?? 'to_request') : null}
              menu={coachMenu}
            />
            <AgentRow group={group} />
            {others.length > 0 && (
              <MemberList
                groupId={groupId}
                members={members}
                currentUserRole={currentUserRole}
                currentUserId={currentUserId}
                isLoading={isMembersLoading}
                // The owner waiting on the seat reads themselves in the coach slot.
                hideSelf={viewer === 'seat'}
              />
            )}
            {viewer === 'member' && <WhoSeesWhat group={group} coachSharing={coachSharing ? coachSharing.consent : null} />}
          </RowBox>
          {/* Getting the athlete in is the owner's second step: the link sits
              right under who is here. Elsewhere it is a setting. */}
          {viewer === 'seat' && isAdmin && <div className="mt-3">{invites(true)}</div>}
        </Section>
      )}

      <AthletePlanModal
        member={viewingPlanFor}
        groupId={groupId}
        onClose={() => setViewingPlanFor(null)}
        onWorkOnPlan={onOpenConversation ? (member) => void handleWorkOnPlan(member) : undefined}
        isOpeningThread={viewingPlanFor !== null && pendingAthleteId === viewingPlanFor.user_id}
      />
      <AthletePhysiologyModal
        member={viewingThresholdsFor}
        groupId={groupId}
        onClose={() => setViewingThresholdsFor(null)}
      />

      {/* Which side of the links the caller is on is the server's answer:
          a coach who is also a member is still the group's coach. */}
      {delegationViewer === 'coach' && (
        <Section headingLevel={3} title={t('delegation.sectionTitle', { platform: rosterPlatform })}>
          <DelegatedConnectionsSection
            groupId={groupId}
            mode="coach"
            connections={delegatedConnections}
            members={members}
            onOpenConnections={onOpenConnections}
          />
        </Section>
      )}

      {viewer === 'coach' && (
        <Section headingLevel={3} title={t('groups.info.agentTitle')} data-testid="group-info-agent-section">
          <RowBox>
            <AgentRow group={group} />
            <ValueRow
              label={t('groups.info.agentReplies')}
              value={t(RESPOND_MODE_LABEL_KEY[group.respond_mode ?? 'all'])}
              data-testid="group-info-agent-replies"
            />
            {weeklyDigest && (
              <ValueRow
                label={t('groups.digestMode')}
                value={t(DIGEST_MODES[group.digest_mode ?? 'off'].labelKey)}
                data-testid="group-info-agent-recap"
              />
            )}
          </RowBox>
        </Section>
      )}

      {showsSharing && (
        <Section headingLevel={3} title={t('groups.info.sharingTitle')} data-testid="group-info-sharing">
          {/* Told once to a sharing member until they answer it here: the coach's AI tools read them by name. */}
          {showsCoachSharing && coachName && coachSharing?.assistant_notice_due && (
            <div className="mb-3">
              <CoachAssistantNotice
                coach={coachName}
                disabled={isSavingCoachSharing}
                onAnswer={(keep) => void handleCoachSharingChange(keep)}
              />
            </div>
          )}
          {/* Asked once of a sharing member until they answer it here: may
              the coach be told of a break unusual for them (carnet#1792). */}
          {showsCoachSharing && coachName && coachConsents?.routine_detection_notice_due && (
            <div className="mb-3">
              <RoutineDetectionNotice
                coach={coachName}
                disabled={isSavingCoachConsents}
                onAnswer={(on) => void handleRoutineDetectionChange(on)}
              />
            </div>
          )}
          <RowBox>
            {showsCoachSharing && (
              <div className={clsx('px-4 py-3', ROW_DIVIDER)} data-testid="coach-sharing-row">
                <Checkbox
                  label={t('humanCoach.shareTraining')}
                  description={
                    group.coach_user_id
                      ? t('humanCoach.shareTrainingHint')
                      : t('humanCoach.shareTrainingPendingHint')
                  }
                  checked={coachSharing?.consent ?? false}
                  disabled={!coachSharing || isSavingCoachSharing}
                  onChange={(e) => void handleCoachSharingChange(e.target.checked)}
                  data-testid="coach-sharing-switch"
                />
              </div>
            )}
            {/* Beside coach sharing: whether the coach's AI assistant (Claude,
                ChatGPT…) includes the member in its roster tools (carnet#1511). */}
            {showsCoachSharing && group.coach_user_id && coachConsents?.assistant_read_supported && (
              <div className={clsx('px-4 py-3', ROW_DIVIDER)} data-testid="coach-assistant-read-row">
                <Checkbox
                  label={t('humanCoach.assistantOptIn.label', { coach: coachName ?? '' })}
                  description={t('humanCoach.assistantOptIn.hint', { coach: coachName ?? '' })}
                  checked={coachConsents.assistant_read}
                  disabled={isSavingCoachConsents}
                  onChange={(e) => void handleAssistantReadChange(e.target.checked)}
                  data-testid="coach-assistant-read-switch"
                />
              </div>
            )}
            {/* Whether the coach's To handle and morning brief say when the
                member's planned sessions go unmatched (carnet#1792): on by
                default, and turned off here without a confirm. */}
            {showsCoachSharing && group.coach_user_id && coachConsents?.missed_session_check_supported && (
              <div className={clsx('px-4 py-3', ROW_DIVIDER)} data-testid="coach-missed-session-check-row">
                <Checkbox
                  label={t('humanCoach.missedSessionCheck.label', { coach: coachName ?? '' })}
                  description={t('humanCoach.missedSessionCheck.hint', { coach: coachName ?? '' })}
                  checked={coachConsents.missed_session_check}
                  disabled={isSavingCoachConsents}
                  onChange={(e) => void handleMissedSessionCheckChange(e.target.checked)}
                  data-testid="coach-missed-session-check-switch"
                />
              </div>
            )}
            {/* Whether the coach's To handle and morning brief say when the
                member's break is longer than any in their own last 8 weeks
                while nothing is planned (carnet#1792): off until granted. */}
            {showsCoachSharing && group.coach_user_id && coachConsents?.routine_detection_supported && (
              <div className={clsx('px-4 py-3', ROW_DIVIDER)} data-testid="coach-routine-detection-row">
                <Checkbox
                  label={t('humanCoach.routineDetection.label', { coach: coachName ?? '' })}
                  description={t('humanCoach.routineDetection.hint', { coach: coachName ?? '' })}
                  checked={coachConsents.routine_detection}
                  disabled={isSavingCoachConsents}
                  onChange={(e) => void handleRoutineDetectionChange(e.target.checked)}
                  data-testid="coach-routine-detection-switch"
                />
              </div>
            )}
            {/* What the coach's agent read of the member, beside the sharing
                that lets it (carnet#1445): only once a human coach is attached. */}
            {showsCoachSharing && group.coach_user_id && (
              <div className={clsx('px-4 py-3', ROW_DIVIDER)}>
                <CoachReadLog groupId={groupId} />
              </div>
            )}
            {/* The caller's own peer-sharing consent, shown while the group
                shares at all: each athlete still decides whether their own
                training is part of it. */}
            {showsPeerConsent && currentMember && (
              <div className={clsx('px-4 py-3', ROW_DIVIDER)} data-testid="peer-consent-card">
                <Checkbox
                  label={t('groups.shareMyData')}
                  description={t('groups.shareMyDataHint')}
                  checked={currentMember.peer_sharing_consent}
                  disabled={isSavingConsent}
                  onChange={(e) => void handleConsentChange(e.target.checked)}
                  data-testid="peer-consent-switch"
                />
              </div>
            )}
          </RowBox>
        </Section>
      )}

      {memberLinks.map((link) => (
        <Section
          key={link.id}
          headingLevel={3}
          title={t('delegation.memberSectionTitle', { platform: coachPlatformName(link.provider) })}
        >
          <DelegatedConnectionsSection groupId={groupId} mode="member" connections={[link]} members={members} />
        </Section>
      ))}

      {/* One block for the coach relationship, never one per link: an athlete who
          connects the platform themselves has no link and still grants here. */}
      {coachConsents && coachName && offeredConsents(coachConsents).length > 0 && (
        <Section headingLevel={3} title={t(COACH_CONSENTS_KEYS.title)}>
          <CoachConsentsSection groupId={groupId} consents={coachConsents} coach={coachName} />
        </Section>
      )}

      {/* The group chat the room is connected to, for everyone once there is
          one; connecting it is the owner's or the coach's (carnet#1683). */}
      <GroupChatConnection groupId={groupId} />

      <RowBox data-testid="group-info-more">
        {showsThisWeek && (
          <DisclosureRow label={t('groups.thisWeek')} data-testid="group-info-this-week">
            <div className="space-y-4">
              {isStatsLoading ? (
                <div className="flex justify-center py-4">
                  <div className="pierre-spinner" />
                </div>
              ) : statsShown && stats ? (
                <div>
                  {garminLine && (
                    <p className="text-xs text-on-surface-variant mb-2" data-testid="group-info-garmin">
                      {garminLine}
                    </p>
                  )}
                  <div className="grid grid-cols-2 gap-3" data-testid="group-info-stats">
                    <div className="stat-card-dark">
                      <p className="text-xs font-medium text-on-surface-variant mb-1">{t('groups.activeMembers')}</p>
                      <p className="text-xl font-bold text-on-surface">{stats.active_members}</p>
                      <p className="text-xs text-outline mt-1">{t('groups.ofTotal', { n: stats.total_members })}</p>
                    </div>
                    <div className="stat-card-dark">
                      <p className="text-xs font-medium text-on-surface-variant mb-1">{t('groups.avgWeeklyVolume')}</p>
                      {/* No member's week read: no figure and no unit, never a zero. */}
                      <p className="text-xl font-bold text-on-surface" data-testid="group-info-avg-volume">
                        {stats.avg_weekly_volume_km !== null ? (
                          <>
                            {weeklyVolumeFigure(language, stats.avg_weekly_volume_km, units)}
                            <span className="text-sm text-on-surface-variant ml-1">{distanceSymbol(units)}</span>
                          </>
                        ) : (
                          '--'
                        )}
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
                  {/* The stats, the weekly report and the health flags all cover the members who share with the reader. */}
                  {statsScopeLine && (
                    <p className="text-xs text-outline mt-2" data-testid="group-info-scope">
                      {statsScopeLine}
                    </p>
                  )}
                </div>
              ) : null}
              <GroupInsightsPanel groupId={groupId} isAdmin={isAdmin} weeklyDigestEnabled={weeklyDigest} />
            </div>
          </DisclosureRow>
        )}
        {showsSettings && (
          <DisclosureRow
            label={t('groups.info.settingsRow')}
            hint={
              !isAdmin
                ? t('groups.digestMode')
                : viewer === 'seat'
                  ? t('groups.info.settingsHintNoInvites')
                  : t('groups.info.settingsHint')
            }
            data-testid="group-info-settings-row"
          >
            <GroupSettingsForm group={group} isAdmin={isAdmin} canSetDigest={canSetDigest}>
              {/* Only an owner or admin may list a group's invites; the route refuses anyone else. */}
              {isAdmin && viewer !== 'seat' && (
                <div className="space-y-2 pt-2">
                  <p className="text-sm font-semibold text-on-surface">{t('groups.tabInvites')}</p>
                  {invites(false)}
                </div>
              )}
            </GroupSettingsForm>
          </DisclosureRow>
        )}
        <DisclosureRow label={t('groups.info.roomRow')} data-testid="group-info-room-row">
          <GroupTranscriptPanel groupId={groupId} />
        </DisclosureRow>
      </RowBox>

      {!isCoachViewer &&
        (isOwner ? (
          <QuietExitRow label={t('groups.info.deleteRow')} onClick={() => setConfirmDelete(true)} data-testid="group-info-delete" />
        ) : (
          <QuietExitRow label={t('groups.info.leaveRow')} onClick={() => setConfirmLeave(true)} data-testid="group-info-leave" />
        ))}

      <ConfirmDialog
        isOpen={confirmLeave}
        onClose={() => setConfirmLeave(false)}
        onConfirm={() => void handleLeave()}
        title={t('groups.info.leaveTitle')}
        message={[
          t('app.confirmLeaveGroupWeb', { group: group.name }),
          // Leaving ends every link the member holds in the group.
          ...memberLinks.map((link) =>
            t('delegation.leaveEndsLink', { platform: coachPlatformName(link.provider) }),
          ),
        ].join(' ')}
        confirmLabel={t('groups.info.leaveConfirm')}
        variant="danger"
        isLoading={isLeaving}
      />

      <ConfirmDialog
        isOpen={confirmDelete}
        onClose={() => setConfirmDelete(false)}
        onConfirm={() => void handleDelete()}
        title={t('groups.info.deleteTitle')}
        message={t('app.confirmArchiveGroupWeb', { group: group.name })}
        confirmLabel={t('groups.info.deleteConfirm')}
        variant="danger"
        isLoading={isDeleting}
      />

      <ConfirmDialog
        isOpen={confirmRemoveCoach}
        onClose={() => setConfirmRemoveCoach(false)}
        onConfirm={() => void handleRemoveCoach()}
        title={t('humanCoach.removeCoachTitle')}
        message={t('humanCoach.detachQ')}
        confirmLabel={t('humanCoach.removeCoachConfirm')}
        variant="danger"
        isLoading={isRemovingCoach}
      />
    </div>
  );
}
