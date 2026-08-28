// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Invite management panel for coaching groups
// ABOUTME: Lists active invites, create new invites with expiry/max uses, copy links, deactivate

import { useState } from 'react';
import { Link2, Plus, Copy, Check, Trash2, UserCog } from 'lucide-react';
import { useGroupInvites, useCreateInvite, useDeactivateInvite } from '../../hooks/useGroups';
import { Button, Card, Select, ConfirmDialog, useErrorToast, useSuccessToast } from '../ui';
import type { SelectOption } from '../ui';
import { useTranslation } from '@pierre/i18n';
import type {
  GroupRole,
  GroupInvite,
  GroupInviteKind,
  CreateInviteRequest,
} from '@pierre/shared-types';

interface InviteManagerProps {
  groupId: string;
  currentUserRole: GroupRole;
}

function expiry_options(t: (key: string) => string): SelectOption[] {
  return [
  { value: '1', label: '1 day' },
  { value: '3', label: '3 days' },
  { value: '7', label: '7 days' },
  { value: '14', label: '14 days' },
  { value: '30', label: '30 days' },
  { value: '0', label: t('settingsUi.neverExpires') },
];
}

function max_uses_options(t: (key: string) => string): SelectOption[] {
  return [
  { value: '1', label: '1 use' },
  { value: '5', label: '5 uses' },
  { value: '10', label: '10 uses' },
  { value: '25', label: '25 uses' },
  { value: '0', label: t('groups.inviteUnlimited') },
];
}

function kind_options(t: (key: string) => string): SelectOption[] {
  return [
  { value: 'member', label: t('groups.inviteTypeMember') },
  { value: 'coach', label: t('groups.coach') },
];
}

function formatDate(dateStr: string): string {
  return new Date(dateStr).toLocaleDateString(undefined, {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  });
}

function isExpired(invite: GroupInvite): boolean {
  if (!invite.expires_at) return false;
  return new Date(invite.expires_at) < new Date();
}

function isExhausted(invite: GroupInvite): boolean {
  if (invite.max_uses === null) return false;
  return invite.use_count >= invite.max_uses;
}

export default function InviteManager({ groupId, currentUserRole }: InviteManagerProps) {
  const { t } = useTranslation();
  const { invites, isLoading } = useGroupInvites(groupId);
  const { createInvite, isPending: isCreating } = useCreateInvite(groupId);
  const { deactivateInvite, isPending: isDeactivating } = useDeactivateInvite(groupId);
  const showError = useErrorToast();
  const showSuccess = useSuccessToast();

  const [showCreateForm, setShowCreateForm] = useState(false);
  const [expiryDays, setExpiryDays] = useState('7');
  const [maxUses, setMaxUses] = useState('10');
  const [inviteKind, setInviteKind] = useState<GroupInviteKind>('member');
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [confirmDeactivate, setConfirmDeactivate] = useState<GroupInvite | null>(null);

  const canManage = currentUserRole === 'owner' || currentUserRole === 'admin';

  const handleCreate = async () => {
    const request: CreateInviteRequest = {};
    const days = parseInt(expiryDays, 10);
    if (days > 0) {
      request.expires_in_days = days;
    }
    const uses = parseInt(maxUses, 10);
    if (uses > 0) {
      request.max_uses = uses;
    }
    if (inviteKind === 'coach') {
      request.kind = 'coach';
    }

    try {
      await createInvite(request);
      const detail =
        inviteKind === 'coach'
          ? t('groups.inviteCoachShare')
          : t('groups.inviteReadyToShare');
      showSuccess(t('app.inviteCreated'), detail);
      setShowCreateForm(false);
    } catch (err) {
      const message = err instanceof Error ? err.message : t('groups.inviteCreateFailed');
      showError(t('app.creationFailed'), message);
    }
  };

  const handleCopyLink = async (code: string, inviteId: string) => {
    const link = `${window.location.origin}/groups/join/${encodeURIComponent(code)}`;
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
      const message = err instanceof Error ? err.message : t('groups.inviteDeactivateFailed');
      showError(t('app.deactivationFailed'), message);
    }
  };

  const activeInvites = invites.filter((inv) => inv.is_active && !isExpired(inv) && !isExhausted(inv));
  const inactiveInvites = invites.filter((inv) => !inv.is_active || isExpired(inv) || isExhausted(inv));

  if (isLoading) {
    return (
      <div className="flex justify-center py-8">
        <div className="pierre-spinner" />
      </div>
    );
  }

  return (
    <div className="space-y-6">
      {/* Header with create button */}
      {canManage && (
        <div className="flex items-center justify-between">
          <p className="text-sm text-on-surface-variant">
            {t('groups.inviteShareHint')}
          </p>
          <Button variant="primary" size="sm" onClick={() => setShowCreateForm(!showCreateForm)}>
            <span className="flex items-center gap-2">
              <Plus className="w-4 h-4" />
              {t('groups.inviteNew')}
            </span>
          </Button>
        </div>
      )}

      {/* Create invite form */}
      {showCreateForm && canManage && (
        <Card variant="dark" className="!p-5">
          <h4 className="text-sm font-semibold text-on-surface mb-4">{t('groups.inviteCreateTitle')}</h4>
          <div className="mb-4">
            <Select
              label={t('groups.inviteType')}
              options={kind_options(t)}
              value={inviteKind}
              onChange={(e) => setInviteKind(e.target.value as GroupInviteKind)}
            />
            {inviteKind === 'coach' && (
              <p className="text-xs text-outline mt-1.5">
                {t('groups.inviteCoachHint')}
              </p>
            )}
          </div>
          <div className="grid grid-cols-2 gap-4 mb-4">
            <Select
              label={t('groups.inviteExpiresAfter')}
              options={expiry_options(t)}
              value={expiryDays}
              onChange={(e) => setExpiryDays(e.target.value)}
            />
            <Select
              label={t('groups.inviteMaxUses')}
              options={max_uses_options(t)}
              value={maxUses}
              onChange={(e) => setMaxUses(e.target.value)}
            />
          </div>
          <div className="flex justify-end gap-2">
            <Button variant="secondary" size="sm" onClick={() => setShowCreateForm(false)}>
              {t('settingsUi.cancel')}
            </Button>
            <Button variant="primary" size="sm" onClick={handleCreate} loading={isCreating}>
              {t('groups.inviteCreate')}
            </Button>
          </div>
        </Card>
      )}

      {/* Active invites */}
      {activeInvites.length > 0 && (
        <div>
          <h4 className="text-sm font-semibold text-on-surface-variant mb-3">
            Active Invites ({activeInvites.length})
          </h4>
          <div className="space-y-2">
            {activeInvites.map((invite) => (
              <div
                key={invite.id}
                className="flex items-center justify-between p-4 rounded-lg bg-surface-container-low hover:bg-surface-container transition-colors"
              >
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2 mb-1">
                    <Link2 className="w-4 h-4 text-primary flex-shrink-0" />
                    <code className="text-sm text-on-surface font-mono truncate">{invite.code}</code>
                    {invite.kind === 'coach' && (
                      <span className="flex items-center gap-1 px-2 py-0.5 text-[10px] font-medium uppercase tracking-wide bg-primary/20 text-primary rounded-full">
                        <UserCog className="w-3 h-3" />
                        {t('groups.coach')}
                      </span>
                    )}
                  </div>
                  <div className="flex items-center gap-3 text-xs text-outline">
                    <span>
                      {invite.use_count} / {invite.max_uses ?? 'unlimited'} uses
                    </span>
                    {invite.expires_at && (
                      <span>{t('frag.expires')} {formatDate(invite.expires_at)}</span>
                    )}
                    {!invite.expires_at && <span>{t('groups.inviteNoExpiry')}</span>}
                  </div>
                </div>
                <div className="flex items-center gap-2 ml-4">
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={() => handleCopyLink(invite.code, invite.id)}
                    title={t('groups.inviteCopy')}
                    aria-label={t('groups.inviteCopyAria')}
                  >
                    {copiedId === invite.id ? (
                      <Check className="w-4 h-4 text-activity" />
                    ) : (
                      <Copy className="w-4 h-4" />
                    )}
                  </Button>
                  {canManage && (
                    <Button
                      variant="danger"
                      size="sm"
                      onClick={() => setConfirmDeactivate(invite)}
                      title={t('groups.inviteDeactivateAria')}
                      aria-label={t('groups.inviteDeactivateThis')}
                    >
                      <Trash2 className="w-4 h-4" />
                    </Button>
                  )}
                </div>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Inactive invites */}
      {inactiveInvites.length > 0 && (
        <div>
          <h4 className="text-sm font-semibold text-on-surface-variant mb-3">
            Expired / Used ({inactiveInvites.length})
          </h4>
          <div className="space-y-2">
            {inactiveInvites.map((invite) => (
              <div
                key={invite.id}
                className="flex items-center justify-between p-4 rounded-lg bg-surface-container-low opacity-50"
              >
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2 mb-1">
                    <Link2 className="w-4 h-4 text-outline flex-shrink-0" />
                    <code className="text-sm text-on-surface-variant font-mono truncate">{invite.code}</code>
                  </div>
                  <div className="flex items-center gap-3 text-xs text-on-surface-variant">
                    <span>{invite.use_count} uses</span>
                    {!invite.is_active && <span>{t('groups.inviteDeactivated')}</span>}
                    {invite.is_active && isExpired(invite) && <span>{t('groups.inviteExpired')}</span>}
                    {invite.is_active && isExhausted(invite) && <span>{t('groups.inviteMaxReached')}</span>}
                  </div>
                </div>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Empty state */}
      {invites.length === 0 && (
        <div className="text-center py-8">
          <Link2 className="w-8 h-8 text-on-surface-variant mx-auto mb-3" />
          <p className="text-outline">{t('groups.inviteEmptyTitle')}</p>
          {canManage && (
            <p className="text-on-surface-variant text-sm mt-1">
              {t('groups.inviteEmptyHint')}
            </p>
          )}
        </div>
      )}

      {/* Confirm deactivation dialog */}
      <ConfirmDialog
        isOpen={!!confirmDeactivate}
        onClose={() => setConfirmDeactivate(null)}
        onConfirm={handleDeactivate}
        title={t('groups.inviteDeactivate')}
        message="This invite link will stop working immediately. Anyone who hasn't used it yet will need a new invite."
        confirmLabel={t('app.deactivate')}
        variant="warning"
        isLoading={isDeactivating}
      />
    </div>
  );
}
