// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A group's live invite links as Group info rows — the code, its uses and expiry, copy, and a `⋯` to deactivate
// ABOUTME: A quiet "New invite link" opens the inline form (expiry, uses, and for a coach with access, approving signups)

import { useState } from 'react';
import { BadgeCheck, Check, Copy, Link2, Plus, UserCog } from 'lucide-react';
import { useGroupInvites, useCreateInvite, useDeactivateInvite } from '../../hooks/useGroups';
import { BADGE_TONE, Button, Checkbox, ConfirmDialog, EmptyState, IconButton, OverflowMenu, Select, useErrorToast, useSuccessToast } from '../ui';
import type { SelectOption } from '../ui';
import { RowBox, ROW_DIVIDER } from './GroupInfoParts';
import { useTranslation } from '@pierre/i18n';
import { useAuth } from '../../hooks/useAuth';
import { formatDateTime, type Translate } from '@pierre/chat-utils';
import { canManageGroup, countKey, INVITE_EXPIRY_DAY_KEYS, INVITE_USE_LIMIT_KEYS } from '@pierre/shared-constants';
import type {
  GroupRole,
  GroupInvite,
  GroupInviteKind,
  CreateInviteRequest,
} from '@pierre/shared-types';
import { describeApiError } from '@pierre/ui-logic';
import { inviteLink } from '../../utils/inviteLink';

interface InviteManagerProps {
  groupId: string;
  currentUserRole: GroupRole;
  /**
   * Offer the coach invite kind. Off for the owner offered the coach seat
   * (`canRequestCoachAccess`): an invite that makes someone else this group's
   * coach contradicts the request they are making.
   */
  offerCoachKind?: boolean;
  /**
   * The form's Create as a quiet button, where another action is the view's
   * one filled button (the coach seat's request in Group info).
   */
  quietCreate?: boolean;
  /** A line above the links: "Invite your athlete" while the owner's group has no one to coach yet. */
  heading?: string;
}

/** The expiry choices, in days; `0` never expires. */
const EXPIRY_DAYS = [1, 3, 7, 14, 30] as const;
/** The use limits; `0` is unlimited. */
const USE_LIMITS = [1, 5, 10, 25] as const;

function expiryOptions(t: Translate, language: string): SelectOption[] {
  return [
    ...EXPIRY_DAYS.map((count) => ({
      value: String(count),
      label: t(countKey(count, language, INVITE_EXPIRY_DAY_KEYS), { count }),
    })),
    { value: '0', label: t('settingsUi.neverExpires') },
  ];
}

function limitOptions(t: Translate, language: string): SelectOption[] {
  return [
    ...USE_LIMITS.map((count) => ({
      value: String(count),
      label: t(countKey(count, language, INVITE_USE_LIMIT_KEYS), { count }),
    })),
    { value: '0', label: t('groups.inviteUnlimited') },
  ];
}

function kindOptions(t: Translate): SelectOption[] {
  return [
    { value: 'member', label: t('groups.inviteTypeMember') },
    { value: 'coach', label: t('humanCoach.coach') },
  ];
}

/**
 * An invite that also approves signups is a coach's signup code (carnet#1128):
 * the server takes it only bounded, at most 50 uses for at most 30 days. Every
 * concrete option above is within both, so the bound drops the unlimited and
 * never-expires choices and falls back to these when one was picked.
 */
const SIGNUP_CODE_DEFAULT_EXPIRY = '7';
const SIGNUP_CODE_DEFAULT_MAX_USES = '10';

function bounded(options: SelectOption[]): SelectOption[] {
  return options.filter((option) => option.value !== '0');
}

function isExpired(invite: GroupInvite): boolean {
  if (!invite.expires_at) return false;
  return new Date(invite.expires_at) < new Date();
}

function isExhausted(invite: GroupInvite): boolean {
  if (invite.max_uses === null) return false;
  return invite.use_count >= invite.max_uses;
}

/**
 * The links that still let someone in. Expired, used-up and deactivated
 * links are not listed: nothing can be done with one, and in a pilot group
 * they outnumbered the people.
 */
export default function InviteManager({
  groupId,
  currentUserRole,
  offerCoachKind = true,
  quietCreate = false,
  heading,
}: InviteManagerProps) {
  const { t, i18n } = useTranslation();
  const { invites, isLoading } = useGroupInvites(groupId);
  const { createInvite, isPending: isCreating } = useCreateInvite(groupId);
  const { deactivateInvite, isPending: isDeactivating } = useDeactivateInvite(groupId);
  const showError = useErrorToast();
  const showSuccess = useSuccessToast();
  // Only a coach with coach access may mint an invite that approves signups;
  // the server enforces it, this only hides a toggle that would be refused.
  const { user } = useAuth();
  const canApproveSignups = user?.manages_roster === true;

  const [showCreateForm, setShowCreateForm] = useState(false);
  const [expiryDays, setExpiryDays] = useState('7');
  const [maxUses, setMaxUses] = useState('10');
  const [inviteKind, setInviteKind] = useState<GroupInviteKind>('member');
  // The kind the invite is made as: a member invite whenever the coach kind is not offered.
  const kind: GroupInviteKind = offerCoachKind ? inviteKind : 'member';
  const [approvesSignup, setApprovesSignup] = useState(false);
  const signupCode = canApproveSignups && kind === 'member' && approvesSignup;
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [confirmDeactivate, setConfirmDeactivate] = useState<GroupInvite | null>(null);

  const canManage = canManageGroup(currentUserRole);
  const language = i18n.language;

  const handleApprovesSignupChange = (checked: boolean) => {
    setApprovesSignup(checked);
    if (checked) {
      // A signup code is always bounded: replace an unlimited choice.
      if (expiryDays === '0') setExpiryDays(SIGNUP_CODE_DEFAULT_EXPIRY);
      if (maxUses === '0') setMaxUses(SIGNUP_CODE_DEFAULT_MAX_USES);
    }
  };

  const handleCreate = async () => {
    const request: CreateInviteRequest = {};
    if (signupCode) {
      request.approves_signup = true;
    }
    // A signup code is always bounded, even if an unlimited choice was made
    // while the form showed a coach invite.
    const bound = (value: string, fallback: string) =>
      signupCode && value === '0' ? fallback : value;
    const days = parseInt(bound(expiryDays, SIGNUP_CODE_DEFAULT_EXPIRY), 10);
    if (days > 0) {
      request.expires_in_days = days;
    }
    const uses = parseInt(bound(maxUses, SIGNUP_CODE_DEFAULT_MAX_USES), 10);
    if (uses > 0) {
      request.max_uses = uses;
    }
    if (kind === 'coach') {
      request.kind = 'coach';
    }

    try {
      await createInvite(request);
      const detail =
        kind === 'coach'
          ? t('humanCoach.inviteShare')
          : t('groups.inviteReadyToShare');
      showSuccess(t('app.inviteCreated'), detail);
      setShowCreateForm(false);
      setApprovesSignup(false);
    } catch (err) {
      const message = describeApiError(err, { t, fallbackKey: 'groups.inviteCreateFailed' });
      showError(t('app.creationFailed'), message);
    }
  };

  const handleCopyLink = async (code: string, inviteId: string) => {
    const link = inviteLink(code);
    try {
      await navigator.clipboard.writeText(link);
      setCopiedId(inviteId);
      showSuccess(t('app.inviteLinkCopied'), t('app.shareThisLink'));
      setTimeout(() => setCopiedId(null), 2000);
    } catch {
      // Fallback: copy just the code
      try {
        await navigator.clipboard.writeText(code);
        setCopiedId(inviteId);
        showSuccess(t('app.inviteCodeCopied'), t('app.shareThisCode'));
        setTimeout(() => setCopiedId(null), 2000);
      } catch {
        showError(t('app.copyFailed'), t('app.couldNotCopyClipboard'));
      }
    }
  };

  const handleDeactivate = async () => {
    if (!confirmDeactivate) return;
    try {
      await deactivateInvite(confirmDeactivate.id);
      showSuccess(t('app.inviteDeactivated'), t('app.inviteLinkStopsWorking'));
      setConfirmDeactivate(null);
    } catch (err) {
      const message = describeApiError(err, { t, fallbackKey: 'groups.inviteDeactivateFailed' });
      showError(t('app.deactivationFailed'), message);
    }
  };

  const activeInvites = invites.filter((inv) => inv.is_active && !isExpired(inv) && !isExhausted(inv));

  if (isLoading) {
    return (
      <div className="flex justify-center py-4">
        <div className="pierre-spinner" />
      </div>
    );
  }

  return (
    <div className="space-y-2" data-testid="invite-manager">
      {heading && <p className="text-sm font-medium text-on-surface">{heading}</p>}

      {activeInvites.length > 0 ? (
        <RowBox>
          {activeInvites.map((invite) => (
            <div key={invite.id} className={`flex items-center gap-3 px-4 py-2.5 ${ROW_DIVIDER}`} data-testid={`invite-row-${invite.id}`}>
              <Link2 className="h-4 w-4 shrink-0 text-primary" aria-hidden="true" />
              <div className="min-w-0 flex-1">
                <div className="flex min-w-0 flex-wrap items-center gap-2">
                  <code className="truncate font-mono text-sm text-on-surface">{invite.code}</code>
                  {invite.kind === 'coach' && (
                    <span className={`inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-xs font-medium ${BADGE_TONE.primary.tone}`}>
                      <UserCog className="h-3 w-3" aria-hidden="true" />
                      {t('humanCoach.coach')}
                    </span>
                  )}
                  {invite.approves_signup && (
                    <span
                      className={`inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-xs font-medium ${BADGE_TONE.primary.tone}`}
                      data-testid="invite-signup-code-badge"
                    >
                      <BadgeCheck className="h-3 w-3" aria-hidden="true" />
                      {t('groups.signupCodeBadge')}
                    </span>
                  )}
                </div>
                <div className="flex flex-wrap items-center gap-x-1.5 text-xs text-on-surface-variant">
                  <span>
                    {t('groups.inviteUses', {
                      used: invite.use_count,
                      max: invite.max_uses ?? t('groups.inviteUnlimited'),
                    })}
                  </span>
                  <span aria-hidden="true">·</span>
                  {invite.expires_at ? (
                    <span>{t('frag.expires')} {formatDateTime(invite.expires_at, language)}</span>
                  ) : (
                    <span>{t('groups.inviteNoExpiry')}</span>
                  )}
                </div>
              </div>
              <IconButton
                onClick={() => void handleCopyLink(invite.code, invite.id)}
                title={t('groups.inviteCopy')}
                aria-label={t('groups.inviteCopyAria')}
              >
                {copiedId === invite.id ? (
                  <Check className="h-4 w-4 text-primary" aria-hidden="true" />
                ) : (
                  <Copy className="h-4 w-4" aria-hidden="true" />
                )}
              </IconButton>
              {canManage && (
                <OverflowMenu
                  label={t('groups.info.inviteMoreActions', { code: invite.code })}
                  items={[
                    {
                      label: t('groups.info.deactivateInvite'),
                      danger: true,
                      onSelect: () => setConfirmDeactivate(invite),
                    },
                  ]}
                />
              )}
            </div>
          ))}
        </RowBox>
      ) : (
        <EmptyState className="!py-1">{t('groups.inviteEmptyTitle')}</EmptyState>
      )}

      {canManage && !showCreateForm && (
        <Button variant="tertiary" onClick={() => setShowCreateForm(true)} data-testid="invite-new">
          <Plus className="mr-1.5 h-4 w-4" aria-hidden="true" />
          {t('groups.info.newInvite')}
        </Button>
      )}

      {showCreateForm && canManage && (
        <RowBox className="space-y-4 p-4" data-testid="invite-create-form">
          <p className="text-sm font-semibold text-on-surface">{t('groups.info.newInvite')}</p>
          {offerCoachKind && (
            <div>
              <Select
                label={t('groups.info.inviteTypeLabel')}
                options={kindOptions(t)}
                value={inviteKind}
                onChange={(e) => setInviteKind(e.target.value as GroupInviteKind)}
              />
              {inviteKind === 'coach' && (
                <p className="mt-1.5 text-xs text-on-surface-variant">
                  {t('humanCoach.inviteHint')}
                </p>
              )}
            </div>
          )}
          {canApproveSignups && kind === 'member' && (
            <Checkbox
              label={t('groups.approvesSignupLabel')}
              description={t('groups.approvesSignupHint')}
              checked={approvesSignup}
              onChange={(e) => handleApprovesSignupChange(e.target.checked)}
            />
          )}
          <div className="grid grid-cols-2 gap-4">
            <Select
              label={t('groups.info.expiresAfterLabel')}
              options={signupCode ? bounded(expiryOptions(t, language)) : expiryOptions(t, language)}
              value={signupCode && expiryDays === '0' ? SIGNUP_CODE_DEFAULT_EXPIRY : expiryDays}
              onChange={(e) => setExpiryDays(e.target.value)}
            />
            <Select
              label={t('groups.info.maxUsesLabel')}
              options={signupCode ? bounded(limitOptions(t, language)) : limitOptions(t, language)}
              value={signupCode && maxUses === '0' ? SIGNUP_CODE_DEFAULT_MAX_USES : maxUses}
              onChange={(e) => setMaxUses(e.target.value)}
            />
          </div>
          <div className="flex justify-end gap-2">
            <Button variant="tertiary" onClick={() => setShowCreateForm(false)}>
              {t('settingsUi.cancel')}
            </Button>
            <Button variant={quietCreate ? 'outline' : 'primary'} onClick={() => void handleCreate()} loading={isCreating}>
              {t('groups.inviteCreate')}
            </Button>
          </div>
        </RowBox>
      )}

      <ConfirmDialog
        isOpen={!!confirmDeactivate}
        onClose={() => setConfirmDeactivate(null)}
        onConfirm={() => void handleDeactivate()}
        title={t('groups.info.deactivateInviteTitle')}
        message={t('groups.info.deactivateInviteBody')}
        confirmLabel={t('app.deactivate')}
        variant="warning"
        isLoading={isDeactivating}
      />
    </div>
  );
}
