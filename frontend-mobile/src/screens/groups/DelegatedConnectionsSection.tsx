// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Group info's coaching-platform links — the coach links roster athletes to members, a member confirms or declines
// ABOUTME: A member grants what their coach may do as they confirm; later, in the coach-consents block. Refusals from details.reason

import React, { useState } from 'react';
import { ActivityIndicator, Alert, Switch, Text, View, type ViewStyle } from 'react-native';
import { useQuery } from '@tanstack/react-query';
import {
  DELEGATION_CONNECTION_REFUSALS,
  DELEGATION_CONSENT_KEYS,
  QUERY_KEYS,
  coachPlatformName,
  confirmConsents,
  consentNoteKey,
  consentStateKeys,
  delegationRefusalKey,
  isDelegationRefusal,
  offeredConsents,
  type DelegationConsent,
} from '@pierre/shared-constants';
import { refusalProvider, refusalReason } from '@pierre/ui-logic';
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
  /**
   * The group's live links the caller may see: all for the coach, their own
   * for a member, one per coaching platform, each answered on its own.
   */
  connections: DelegatedConnection[];
  /** The group's live members, the coach's picker's choices. */
  members: GroupMember[];
  /** Open the connections pane, where the coach's own coaching platform lives. */
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

/** Says a failed step from its refusal reason, naming the platform it is about. */
function useRefusalAlert() {
  const { t } = useTranslation();
  return (err: unknown) => {
    const platform = coachPlatformName(refusalProvider(err));
    Alert.alert(t('common.error'), t(delegationRefusalKey(refusalReason(err)), { platform }));
  };
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
    <View className="gap-4">
      {connections.map((link) => (
        // Keyed by the link, so a grant switched on for one proposal never carries over to the next.
        <View key={link.id} testID={`delegation-member-link-${link.id}`}>
          <MemberLink groupId={groupId} link={link} />
        </View>
      ))}
    </View>
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
  const { athletes, provider, isLoading, isError, error } = useDelegationRoster(groupId, true);
  const { refreshRoster, isPending: isRefreshing } = useRefreshDelegationRoster(groupId);
  const { proposeLink } = useProposeDelegatedConnection(groupId);
  const { endLink } = useEndDelegatedConnection(groupId);
  const [pickingFor, setPickingFor] = useState<DelegationRosterAthlete | null>(null);

  const linkedMemberIds = new Set(connections.map((link) => link.member_user_id));
  const choices = members.filter((m) => !linkedMemberIds.has(m.user_id));
  const reason = isError ? refusalReason(error) : undefined;
  // The roster names its platform; a refused read names the one it is about;
  // a failed read falls back to the platform the coach's links already run on.
  const platform = coachPlatformName(
    provider ?? (isError ? refusalProvider(error) : undefined) ?? connections[0]?.provider,
  );
  const nameOf = (member: GroupMember | undefined, fallback: string) => member?.display_name ?? fallback;

  const propose = async (athlete: DelegationRosterAthlete, member: GroupMember) => {
    setPickingFor(null);
    if (!provider) return;
    try {
      await proposeLink({ provider, athleteId: athlete.provider_athlete_id, memberUserId: member.user_id });
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
    Alert.alert(t('delegation.unlinkTitle', { platform }), t('delegation.unlinkBody', { platform }), [
      { text: t('common.cancel'), style: 'cancel' },
      { text: t('delegation.unlink'), style: 'destructive', onPress: () => void end(link) },
    ]);
  };

  let body: React.ReactNode;
  if (isLoading) {
    body = (
      <View className="flex-row items-center gap-2 py-2">
        <ActivityIndicator size="small" color={colors.tokens.primary} />
        <Text className="text-sm text-text-secondary">{t('delegation.rosterLoading', { platform })}</Text>
      </View>
    );
  } else if (isError) {
    const refused = isDelegationRefusal(reason);
    body = (
      <View className="py-2 gap-1" testID="delegation-roster-refused">
        <Text className="text-sm text-text-secondary">
          {refused ? t(delegationRefusalKey(reason), { platform }) : t('delegation.rosterFailed', { platform })}
        </Text>
        {refused && DELEGATION_CONNECTION_REFUSALS.has(reason) && onOpenConnections && (
          <InkAction label={t('delegation.openConnections')} onPress={onOpenConnections} testID="delegation-open-connections" />
        )}
      </View>
    );
  } else if (athletes.length === 0) {
    body = <Text className="text-sm text-text-tertiary py-2">{t('delegation.rosterEmpty', { platform })}</Text>;
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
            // The state line, then what the member granted beyond the read,
            // each on a line of its own.
            subtitle = [
              link.read_refused
                ? t(delegationRefusalKey(link.read_refused), { platform })
                : t('delegation.linkedTo', { member: linkedName }),
              ...consentStateKeys(link).map((key) => t(key)),
            ].join('\n');
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
                    {link && (
                      <StatusDot tone={link.status === 'confirmed' && !link.read_refused ? 'success' : 'warning'} />
                    )}
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
      <Text className="text-sm text-text-secondary py-2">{t('delegation.coachHint', { platform })}</Text>
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

function MemberLink({ groupId, link }: { groupId: string; link: DelegatedConnection }) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const platform = coachPlatformName(link.provider);
  const showRefusal = useRefusalAlert();
  const { confirmLink, isPending: isConfirming } = useConfirmDelegatedConnection(groupId);
  const { endLink, isPending: isEnding } = useEndDelegatedConnection(groupId);
  // What the member grants with their confirm: nothing until they switch it on.
  const [grants, setGrants] = useState<Partial<Record<DelegationConsent, boolean>>>({});

  // Whether the coach must sign in again is a fact of the coach's connection,
  // which the provider row carries; the group's link list does not.
  const isConfirmed = link.status === 'confirmed';
  const { data: providers } = useQuery({
    queryKey: QUERY_KEYS.providers.status(),
    queryFn: () => oauthApi.getProvidersStatus(),
    enabled: isConfirmed,
  });
  const coachNeedsReauth =
    providers?.providers.some(
      (p) => p.delegation?.connection_id === link.id && p.delegation?.coach_needs_reauth === true,
    ) ?? false;

  const coach = link.coach_display_name;
  const offered = offeredConsents(link);

  const confirm = async () => {
    try {
      await confirmLink(link.id, confirmConsents(link, grants));
    } catch (err) {
      showRefusal(err);
    }
  };

  /**
   * A row with a switch for each consent the link offers, on as `isGranted`
   * says, its hint followed by why a granted one syncs nothing: what the
   * member grants with their confirm. Once confirmed, they change it in the
   * group's coach-consents block, since it lives on their coach relationship.
   */
  const consentRows = (
    isGranted: (consent: DelegationConsent) => boolean,
    onChange: (consent: DelegationConsent, granted: boolean) => void,
    disabled: boolean,
  ) =>
    offered.length > 0 && (
      <View style={CANCEL_PANEL_INSET} testID="delegation-consents">
        {offered.map((consent, index) => {
          const label = t(DELEGATION_CONSENT_KEYS[consent].label, { platform, coach });
          const note = consentNoteKey(link, consent);
          const hint = t(DELEGATION_CONSENT_KEYS[consent].hint, { platform, coach });
          return (
            <Row
              key={consent}
              title={label}
              subtitle={note ? `${hint}\n${t(note, { platform })}` : hint}
              trailing={
                <Switch
                  value={isGranted(consent)}
                  onValueChange={(granted) => onChange(consent, granted)}
                  disabled={disabled}
                  trackColor={{ false: colors.border.default, true: colors.tokens.primary }}
                  accessibilityLabel={label}
                  testID={`delegation-consent-${consent}`}
                />
              }
              last={index === offered.length - 1}
              testID={`delegation-consent-row-${consent}`}
            />
          );
        })}
      </View>
    );

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
        <Text className="text-base font-semibold text-text-primary">{t('delegation.requestTitle', { platform })}</Text>
        <Text className="text-sm text-text-secondary" testID="delegation-request-body">
          {link.provider_athlete_name
            ? t('delegation.requestBody', { platform, coach, athlete: link.provider_athlete_name })
            : t('delegation.requestBodyNoName', { platform, coach })}
        </Text>
        {consentRows(
          (consent) => grants[consent] === true,
          (consent, granted) => setGrants((held) => ({ ...held, [consent]: granted })),
          isConfirming || isEnding,
        )}
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
    Alert.alert(t('delegation.unlinkTitle', { platform }), t('delegation.unlinkBody', { platform }), [
      { text: t('common.cancel'), style: 'cancel' },
      { text: t('delegation.unlink'), style: 'destructive', onPress: () => void end() },
    ]);
  };

  return (
    <View testID="delegation-linked">
      <View className="flex-row items-center gap-3 py-2">
        {link.read_refused ? (
          <StatusLine tone="warning" text={t(delegationRefusalKey(link.read_refused), { platform })} testID="delegation-read-refused" />
        ) : (
          <StatusLine
            tone={coachNeedsReauth ? 'warning' : 'success'}
            text={coachNeedsReauth ? t('delegation.coachReconnectNeeded', { platform, coach }) : t('delegation.confirmedBody', { platform, coach })}
          />
        )}
        <InkAction label={t('delegation.unlink')} onPress={unlink} testID={`delegation-unlink-${link.id}`} />
      </View>
    </View>
  );
}
