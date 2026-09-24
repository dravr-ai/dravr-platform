// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Group info's TrainingPeaks links — the coach links roster athletes to members, a member confirms or declines
// ABOUTME: Either side ends a link; every refusal is worded from its details.reason, never from the server's English

import React, { useState } from 'react';
import { ActivityIndicator, Alert, Text, View, type ViewStyle } from 'react-native';
import { useQuery } from '@tanstack/react-query';
import {
  DELEGATION_CONNECTION_REFUSALS,
  QUERY_KEYS,
  delegationRefusalKey,
  isDelegationRefusal,
} from '@pierre/shared-constants';
import { refusalReason } from '@pierre/ui-logic';
import { useTranslation } from '@pierre/i18n';
import { useThemeColors } from '../../constants/theme';
import { Button, Row, Sheet, StatusDot } from '../../components/ui';
import {
  useConfirmDelegatedConnection,
  useDelegationRoster,
  useEndDelegatedConnection,
  useProposeDelegatedConnection,
  useRefreshDelegationRoster,
} from '../../hooks/useGroups';
import { oauthApi } from '../../services/api';
import type { DelegatedConnection, DelegationRosterAthlete, GroupMember } from '../../types';

/** Cancels the host sheet's side inset, so a `Row` pays its own (see GroupInfoSheet). */
const CANCEL_PANEL_INSET: ViewStyle = { marginHorizontal: -16 };

export interface DelegatedConnectionsSectionProps {
  groupId: string;
  /** The coach links athletes; a member answers the link naming them. */
  mode: 'coach' | 'member';
  /** The group's live links the caller may see: all for the coach, their own for a member. */
  connections: DelegatedConnection[];
  /** The group's live members, the coach's picker's choices. */
  members: GroupMember[];
  /** Open the connections pane, where the coach's own TrainingPeaks lives. */
  onOpenConnections?: () => void;
}

/** The ink word a row's one action reads as. */
function InkAction({ label, onPress, testID }: { label: string; onPress: () => void; testID: string }) {
  return (
    <Text className="text-md font-medium text-primary" onPress={onPress} accessibilityRole="button" testID={testID}>
      {label}
    </Text>
  );
}

/** A state line: a coloured dot and the words, the dot never the only signal. */
function StatusLine({ tone, text, testID }: { tone: 'success' | 'warning'; text: string; testID?: string }) {
  return (
    <View className="flex-row items-center gap-2 flex-1 min-w-0" testID={testID}>
      <StatusDot tone={tone} />
      <Text className="text-sm text-text-secondary flex-1">{text}</Text>
    </View>
  );
}

/** Says a failed step from its refusal reason. */
function useRefusalAlert() {
  const { t } = useTranslation();
  return (err: unknown) => Alert.alert(t('common.error'), t(delegationRefusalKey(refusalReason(err))));
}

export function DelegatedConnectionsSection({
  groupId,
  mode,
  connections,
  members,
  onOpenConnections,
}: DelegatedConnectionsSectionProps) {
  return mode === 'coach' ? (
    <CoachLinks groupId={groupId} connections={connections} members={members} onOpenConnections={onOpenConnections} />
  ) : (
    <MemberLink groupId={groupId} link={connections[0] ?? null} />
  );
}

// ============================================================================
// Coach
// ============================================================================

function CoachLinks({
  groupId,
  connections,
  members,
  onOpenConnections,
}: Omit<DelegatedConnectionsSectionProps, 'mode'>) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const showRefusal = useRefusalAlert();
  const { athletes, isLoading, isError, error } = useDelegationRoster(groupId, true);
  const { refreshRoster, isPending: isRefreshing } = useRefreshDelegationRoster(groupId);
  const { proposeLink } = useProposeDelegatedConnection(groupId);
  const { endLink } = useEndDelegatedConnection(groupId);
  const [pickingFor, setPickingFor] = useState<DelegationRosterAthlete | null>(null);

  const linkedMemberIds = new Set(connections.map((link) => link.member_user_id));
  const choices = members.filter((m) => !linkedMemberIds.has(m.user_id));
  const reason = isError ? refusalReason(error) : undefined;
  const nameOf = (member: GroupMember | undefined, fallback: string) => member?.display_name ?? fallback;

  const propose = async (athlete: DelegationRosterAthlete, member: GroupMember) => {
    setPickingFor(null);
    try {
      await proposeLink({ athleteId: athlete.provider_athlete_id, memberUserId: member.user_id });
    } catch (err) {
      showRefusal(err);
    }
  };

  const end = async (link: DelegatedConnection) => {
    try {
      await endLink(link.id);
    } catch (err) {
      showRefusal(err);
    }
  };

  const confirmUnlink = (link: DelegatedConnection) => {
    Alert.alert(t('delegation.unlinkTitle'), t('delegation.unlinkBody'), [
      { text: t('common.cancel'), style: 'cancel' },
      { text: t('delegation.unlink'), style: 'destructive', onPress: () => void end(link) },
    ]);
  };

  let body: React.ReactNode;
  if (isLoading) {
    body = (
      <View className="flex-row items-center gap-2 py-2">
        <ActivityIndicator size="small" color={colors.tokens.primary} />
        <Text className="text-sm text-text-secondary">{t('delegation.rosterLoading')}</Text>
      </View>
    );
  } else if (isError) {
    const refused = isDelegationRefusal(reason);
    body = (
      <View className="py-2 gap-1" testID="delegation-roster-refused">
        <Text className="text-sm text-text-secondary">
          {refused ? t(delegationRefusalKey(reason)) : t('delegation.rosterFailed')}
        </Text>
        {refused && DELEGATION_CONNECTION_REFUSALS.has(reason) && onOpenConnections && (
          <InkAction label={t('delegation.openConnections')} onPress={onOpenConnections} testID="delegation-open-connections" />
        )}
      </View>
    );
  } else if (athletes.length === 0) {
    body = <Text className="text-sm text-text-tertiary py-2">{t('delegation.rosterEmpty')}</Text>;
  } else {
    body = (
      <View style={CANCEL_PANEL_INSET}>
        {athletes.map((athlete, index) => {
          const athleteId = athlete.provider_athlete_id;
          const link = athlete.connection;
          const linkedName = link
            ? nameOf(members.find((m) => m.user_id === link.member_user_id), link.member_display_name)
            : '';
          let trailing: React.ReactNode;
          let subtitle: string | undefined;
          if (link?.status === 'confirmed') {
            subtitle = t('delegation.linkedTo', { member: linkedName });
            trailing = (
              <InkAction label={t('delegation.unlink')} onPress={() => confirmUnlink(link)} testID={`delegation-unlink-${link.id}`} />
            );
          } else if (link) {
            subtitle = t('delegation.waitingFor', { member: linkedName });
            trailing = (
              <InkAction label={t('delegation.withdraw')} onPress={() => void end(link)} testID={`delegation-withdraw-${link.id}`} />
            );
          } else {
            trailing = (
              <InkAction label={t('delegation.propose')} onPress={() => setPickingFor(athlete)} testID={`delegation-propose-${athleteId}`} />
            );
          }
          return (
            <View key={athleteId}>
              <Row
                title={athlete.display_name ?? athleteId}
                subtitle={subtitle}
                trailing={
                  <View className="flex-row items-center gap-2">
                    {link && <StatusDot tone={link.status === 'confirmed' ? 'success' : 'warning'} />}
                    {trailing}
                  </View>
                }
                last={index === athletes.length - 1}
                testID={`delegation-roster-row-${athleteId}`}
              />
            </View>
          );
        })}
      </View>
    );
  }

  const canRefresh = !isLoading && (!isError || !isDelegationRefusal(reason));

  return (
    <View testID="delegation-section">
      <Text className="text-sm text-text-secondary py-2">{t('delegation.coachHint')}</Text>
      {body}
      {canRefresh && (
        <View className="py-2">
          {isRefreshing ? (
            <ActivityIndicator size="small" color={colors.tokens.primary} />
          ) : (
            <InkAction
              label={t('delegation.refresh')}
              onPress={() => void refreshRoster().catch(showRefusal)}
              testID="delegation-refresh"
            />
          )}
        </View>
      )}

      <Sheet visible={pickingFor !== null} onClose={() => setPickingFor(null)} flush testID="delegation-member-picker">
        <Text className="text-base font-semibold text-text-primary px-4 pb-2">{t('delegation.memberPlaceholder')}</Text>
        {pickingFor &&
          choices.map((member, index) => {
            const name = nameOf(member, t('app.thisMember'));
            return (
              <Row
                key={member.user_id}
                title={
                  member.user_id === pickingFor.suggested_member_user_id
                    ? t('delegation.suggested', { member: name })
                    : name
                }
                onPress={() => void propose(pickingFor, member)}
                last={index === choices.length - 1}
                testID={`delegation-member-option-${member.user_id}`}
              />
            );
          })}
      </Sheet>
    </View>
  );
}

// ============================================================================
// Member
// ============================================================================

function MemberLink({ groupId, link }: { groupId: string; link: DelegatedConnection | null }) {
  const { t } = useTranslation();
  const showRefusal = useRefusalAlert();
  const { confirmLink, isPending: isConfirming } = useConfirmDelegatedConnection(groupId);
  const { endLink, isPending: isEnding } = useEndDelegatedConnection(groupId);

  // Whether the coach must sign in again is a fact of the coach's connection,
  // which the provider row carries; the group's link list does not.
  const isConfirmed = link?.status === 'confirmed';
  const { data: providers } = useQuery({
    queryKey: QUERY_KEYS.providers.status(),
    queryFn: () => oauthApi.getProvidersStatus(),
    enabled: isConfirmed,
  });
  const coachNeedsReauth =
    providers?.providers.some(
      (p) => p.delegation?.connection_id === link?.id && p.delegation?.coach_needs_reauth === true,
    ) ?? false;

  if (!link) return null;
  const coach = link.coach_display_name;

  const confirm = async () => {
    try {
      await confirmLink(link.id);
    } catch (err) {
      showRefusal(err);
    }
  };

  const end = async () => {
    try {
      await endLink(link.id);
    } catch (err) {
      showRefusal(err);
    }
  };

  if (!isConfirmed) {
    return (
      <View className="py-2 gap-2" testID="delegation-request">
        <Text className="text-base font-semibold text-text-primary">{t('delegation.requestTitle')}</Text>
        <Text className="text-sm text-text-secondary" testID="delegation-request-body">
          {link.provider_athlete_name
            ? t('delegation.requestBody', { coach, athlete: link.provider_athlete_name })
            : t('delegation.requestBodyNoName', { coach })}
        </Text>
        <Text className="text-xs text-text-tertiary">{t('delegation.requestFootnote')}</Text>
        <View className="flex-row gap-2 pt-1">
          <Button
            title={t('delegation.confirm')}
            onPress={() => void confirm()}
            loading={isConfirming}
            disabled={isEnding}
            testID="delegation-confirm"
          />
          <Button
            title={t('delegation.decline')}
            variant="ghost"
            onPress={() => void end()}
            loading={isEnding}
            disabled={isConfirming}
            testID="delegation-decline"
          />
        </View>
      </View>
    );
  }

  const unlink = () => {
    Alert.alert(t('delegation.unlinkTitle'), t('delegation.unlinkBody'), [
      { text: t('common.cancel'), style: 'cancel' },
      { text: t('delegation.unlink'), style: 'destructive', onPress: () => void end() },
    ]);
  };

  return (
    <View className="flex-row items-center gap-3 py-2" testID="delegation-linked">
      <StatusLine
        tone={coachNeedsReauth ? 'warning' : 'success'}
        text={coachNeedsReauth ? t('delegation.coachReconnectNeeded', { coach }) : t('delegation.confirmedBody', { coach })}
      />
      <InkAction label={t('delegation.unlink')} onPress={unlink} testID={`delegation-unlink-${link.id}`} />
    </View>
  );
}
