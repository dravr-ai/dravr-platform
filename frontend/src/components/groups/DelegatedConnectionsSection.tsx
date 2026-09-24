// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Group info's TrainingPeaks links — the coach links roster athletes to members, a member confirms or declines
// ABOUTME: Either side ends a link; every refusal is worded from its details.reason, never from the server's English

import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import type { DelegatedConnection, DelegationRosterAthlete, GroupMember } from '@pierre/shared-types';
import {
  DELEGATION_CONNECTION_REFUSALS,
  QUERY_KEYS,
  delegationRefusalKey,
  isDelegationRefusal,
} from '@pierre/shared-constants';
import { refusalReason } from '@pierre/ui-logic';
import { useTranslation } from '@pierre/i18n';
import {
  useConfirmDelegatedConnection,
  useDelegationRoster,
  useEndDelegatedConnection,
  useProposeDelegatedConnection,
  useRefreshDelegationRoster,
} from '../../hooks/useGroups';
import { providersApi } from '../../services/api';
import { Button, ConfirmDialog, Select, useErrorToast, useSuccessToast } from '../ui';

interface DelegatedConnectionsSectionProps {
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

/** A state line: a coloured dot and the words, the dot never the only signal. */
function StatusLine({ tone, children, testId }: { tone: 'success' | 'warning'; children: React.ReactNode; testId?: string }) {
  return (
    <span className="inline-flex items-center gap-1.5 text-xs text-on-surface-variant" data-testid={testId}>
      <span aria-hidden="true" className={`h-2 w-2 flex-shrink-0 rounded-full ${tone === 'success' ? 'bg-success' : 'bg-warning'}`} />
      {children}
    </span>
  );
}

/** How a member is named in the picker and the state lines. */
function memberName(member: GroupMember | undefined, fallback: string): string {
  return member?.display_name ?? fallback;
}

export default function DelegatedConnectionsSection({
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

/** Words a failed step from its refusal reason, as a toast. */
function useRefusalToast() {
  const { t } = useTranslation();
  const showError = useErrorToast();
  return (err: unknown) => showError(t(delegationRefusalKey(refusalReason(err))));
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
  const { athletes, isLoading, isError, error } = useDelegationRoster(groupId, true);
  const { refreshRoster, isPending: isRefreshing } = useRefreshDelegationRoster(groupId);
  const showRefusal = useRefusalToast();

  const linkedMemberIds = new Set(connections.map((link) => link.member_user_id));
  const reason = isError ? refusalReason(error) : undefined;

  let body: React.ReactNode;
  if (isLoading) {
    body = (
      <div className="flex items-center gap-2 py-2 text-sm text-on-surface-variant">
        <div className="pierre-spinner w-4 h-4" />
        {t('delegation.rosterLoading')}
      </div>
    );
  } else if (isError) {
    const refused = isDelegationRefusal(reason);
    body = (
      <div className="space-y-1" data-testid="delegation-roster-refused">
        <p className="text-sm text-on-surface-variant">
          {refused ? t(delegationRefusalKey(reason)) : t('delegation.rosterFailed')}
        </p>
        {refused && DELEGATION_CONNECTION_REFUSALS.has(reason) && onOpenConnections && (
          <Button variant="tertiary" size="sm" onClick={onOpenConnections} data-testid="delegation-open-connections">
            {t('delegation.openConnections')}
          </Button>
        )}
      </div>
    );
  } else if (athletes.length === 0) {
    body = <p className="text-sm text-outline">{t('delegation.rosterEmpty')}</p>;
  } else {
    body = (
      <div>
        {athletes.map((athlete) => (
          <RosterRow
            key={athlete.provider_athlete_id}
            groupId={groupId}
            athlete={athlete}
            members={members.filter((m) => !linkedMemberIds.has(m.user_id))}
            allMembers={members}
          />
        ))}
      </div>
    );
  }

  // A roster the coach cannot read yet has nothing to refresh, except a
  // transient failure, which a live read may cure.
  const canRefresh = !isLoading && (!isError || !isDelegationRefusal(reason));

  return (
    <div className="space-y-2" data-testid="delegation-section">
      <p className="text-sm text-on-surface-variant">{t('delegation.coachHint')}</p>
      {body}
      {canRefresh && (
        <Button
          variant="tertiary"
          size="sm"
          loading={isRefreshing}
          onClick={() => void refreshRoster().catch(showRefusal)}
          data-testid="delegation-refresh"
        >
          {t('delegation.refresh')}
        </Button>
      )}
    </div>
  );
}

function RosterRow({
  groupId,
  athlete,
  members,
  allMembers,
}: {
  groupId: string;
  athlete: DelegationRosterAthlete;
  /** Members no live link holds: the picker's choices. */
  members: GroupMember[];
  /** Every live member, to name the one a link holds. */
  allMembers: GroupMember[];
}) {
  const { t } = useTranslation();
  const showSuccess = useSuccessToast();
  const showRefusal = useRefusalToast();
  const { proposeLink, isPending: isProposing } = useProposeDelegatedConnection(groupId);
  const { endLink, isPending: isEnding } = useEndDelegatedConnection(groupId);
  const suggested = members.some((m) => m.user_id === athlete.suggested_member_user_id)
    ? (athlete.suggested_member_user_id as string)
    : '';
  // The coach's own pick wins; until they make one, the picker follows the
  // suggestion, which can only name a member once the members have loaded.
  const [picked, setPicked] = useState<string | null>(null);
  const selected = picked ?? suggested;
  const [confirmUnlink, setConfirmUnlink] = useState(false);

  const athleteId = athlete.provider_athlete_id;
  const athleteName = athlete.display_name ?? athleteId;
  const link = athlete.connection;
  const linkedName = link
    ? memberName(allMembers.find((m) => m.user_id === link.member_user_id), link.member_display_name)
    : '';

  const propose = async () => {
    const member = members.find((m) => m.user_id === selected);
    if (!member) return;
    try {
      await proposeLink({ athleteId, memberUserId: member.user_id });
      showSuccess(
        t('delegation.proposedToast'),
        t('delegation.proposedBody', { member: memberName(member, t('app.thisMember')) }),
      );
    } catch (err) {
      showRefusal(err);
    }
  };

  const end = async () => {
    if (!link) return;
    try {
      await endLink(link.id);
      showSuccess(t('delegation.unlinked'));
      setConfirmUnlink(false);
    } catch (err) {
      showRefusal(err);
    }
  };

  return (
    <div
      className="border-t ghost-border-faint py-3 first:border-t-0 first:pt-0"
      data-testid={`delegation-roster-row-${athleteId}`}
    >
      <p className="text-sm text-on-surface">{athleteName}</p>
      {link?.status === 'confirmed' ? (
        <div className="mt-1 flex items-center justify-between gap-3">
          <StatusLine tone="success">{t('delegation.linkedTo', { member: linkedName })}</StatusLine>
          <Button
            variant="tertiary"
            size="sm"
            className="text-error"
            onClick={() => setConfirmUnlink(true)}
            data-testid={`delegation-unlink-${link.id}`}
          >
            {t('delegation.unlink')}
          </Button>
        </div>
      ) : link ? (
        <div className="mt-1 flex items-center justify-between gap-3">
          <StatusLine tone="warning" testId={`delegation-waiting-${link.id}`}>
            {t('delegation.waitingFor', { member: linkedName })}
          </StatusLine>
          <Button
            variant="tertiary"
            size="sm"
            loading={isEnding}
            onClick={() => void end()}
            data-testid={`delegation-withdraw-${link.id}`}
          >
            {t('delegation.withdraw')}
          </Button>
        </div>
      ) : (
        <div className="mt-1 flex items-end gap-3">
          <div className="min-w-0 flex-1">
            <Select
              id={`delegation-member-select-${athleteId}`}
              aria-label={t('delegation.memberLabel')}
              size="sm"
              value={selected}
              placeholder={t('delegation.memberPlaceholder')}
              onChange={(e) => setPicked(e.target.value)}
              options={members.map((m) => {
                const name = memberName(m, t('app.thisMember'));
                return {
                  value: m.user_id,
                  label: m.user_id === athlete.suggested_member_user_id ? t('delegation.suggested', { member: name }) : name,
                };
              })}
              data-testid={`delegation-member-select-${athleteId}`}
            />
          </div>
          <Button
            variant="secondary"
            size="sm"
            disabled={!selected}
            loading={isProposing}
            onClick={() => void propose()}
            data-testid={`delegation-propose-${athleteId}`}
          >
            {t('delegation.propose')}
          </Button>
        </div>
      )}

      <ConfirmDialog
        isOpen={confirmUnlink}
        onClose={() => setConfirmUnlink(false)}
        onConfirm={() => void end()}
        title={t('delegation.unlinkTitle')}
        message={t('delegation.unlinkBody')}
        confirmLabel={t('delegation.unlink')}
        variant="danger"
        isLoading={isEnding}
      />
    </div>
  );
}

// ============================================================================
// Member
// ============================================================================

function MemberLink({ groupId, link }: { groupId: string; link: DelegatedConnection | null }) {
  const { t } = useTranslation();
  const showSuccess = useSuccessToast();
  const showRefusal = useRefusalToast();
  const { confirmLink, isPending: isConfirming } = useConfirmDelegatedConnection(groupId);
  const { endLink, isPending: isEnding } = useEndDelegatedConnection(groupId);
  const [confirmUnlink, setConfirmUnlink] = useState(false);

  // Whether the coach must sign in again is a fact of the coach's connection,
  // which the provider card carries; the group's link list does not.
  const isConfirmed = link?.status === 'confirmed';
  const { data: providers } = useQuery({
    queryKey: QUERY_KEYS.providers.status(),
    queryFn: () => providersApi.getProvidersStatus(),
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
      showSuccess(t('delegation.confirmedToast'), t('delegation.confirmedBody', { coach }));
    } catch (err) {
      showRefusal(err);
    }
  };

  const end = async (declining: boolean) => {
    try {
      await endLink(link.id);
      showSuccess(declining ? t('delegation.declinedToast') : t('delegation.unlinked'));
      setConfirmUnlink(false);
    } catch (err) {
      showRefusal(err);
    }
  };

  if (!isConfirmed) {
    return (
      <div className="space-y-2" data-testid="delegation-request">
        <p className="text-sm font-medium text-on-surface">{t('delegation.requestTitle')}</p>
        <p className="text-sm text-on-surface-variant">
          {link.provider_athlete_name
            ? t('delegation.requestBody', { coach, athlete: link.provider_athlete_name })
            : t('delegation.requestBodyNoName', { coach })}
        </p>
        <p className="text-xs text-outline">{t('delegation.requestFootnote')}</p>
        <div className="flex items-center gap-2 pt-1">
          <Button
            variant="primary"
            size="sm"
            loading={isConfirming}
            disabled={isEnding}
            onClick={() => void confirm()}
            data-testid="delegation-confirm"
          >
            {t('delegation.confirm')}
          </Button>
          <Button
            variant="tertiary"
            size="sm"
            loading={isEnding}
            disabled={isConfirming}
            onClick={() => void end(true)}
            data-testid="delegation-decline"
          >
            {t('delegation.decline')}
          </Button>
        </div>
      </div>
    );
  }

  return (
    <div className="flex items-center justify-between gap-3" data-testid="delegation-linked">
      {coachNeedsReauth ? (
        <StatusLine tone="warning">{t('delegation.coachReconnectNeeded', { coach })}</StatusLine>
      ) : (
        <StatusLine tone="success">{t('delegation.confirmedBody', { coach })}</StatusLine>
      )}
      <Button
        variant="tertiary"
        size="sm"
        className="text-error"
        onClick={() => setConfirmUnlink(true)}
        data-testid={`delegation-unlink-${link.id}`}
      >
        {t('delegation.unlink')}
      </Button>
      <ConfirmDialog
        isOpen={confirmUnlink}
        onClose={() => setConfirmUnlink(false)}
        onConfirm={() => void end(false)}
        title={t('delegation.unlinkTitle')}
        message={t('delegation.unlinkBody')}
        confirmLabel={t('delegation.unlink')}
        variant="danger"
        isLoading={isEnding}
      />
    </div>
  );
}
