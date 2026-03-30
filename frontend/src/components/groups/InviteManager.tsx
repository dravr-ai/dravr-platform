// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Invite management panel for coaching groups
// ABOUTME: Lists active invites, create new invites with expiry/max uses, copy links, deactivate

import { useState } from 'react';
import { Link2, Plus, Copy, Check, Trash2 } from 'lucide-react';
import { useGroupInvites, useCreateInvite, useDeactivateInvite } from '../../hooks/useGroups';
import { Button, Card, Select, ConfirmDialog, useErrorToast, useSuccessToast } from '../ui';
import type { SelectOption } from '../ui';
import type { GroupRole, GroupInvite, CreateInviteRequest } from '@pierre/shared-types';

interface InviteManagerProps {
  groupId: string;
  currentUserRole: GroupRole;
}

const EXPIRY_OPTIONS: SelectOption[] = [
  { value: '1', label: '1 day' },
  { value: '3', label: '3 days' },
  { value: '7', label: '7 days' },
  { value: '14', label: '14 days' },
  { value: '30', label: '30 days' },
  { value: '0', label: 'Never expires' },
];

const MAX_USES_OPTIONS: SelectOption[] = [
  { value: '1', label: '1 use' },
  { value: '5', label: '5 uses' },
  { value: '10', label: '10 uses' },
  { value: '25', label: '25 uses' },
  { value: '0', label: 'Unlimited' },
];

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
  const { invites, isLoading } = useGroupInvites(groupId);
  const { createInvite, isPending: isCreating } = useCreateInvite(groupId);
  const { deactivateInvite, isPending: isDeactivating } = useDeactivateInvite(groupId);
  const showError = useErrorToast();
  const showSuccess = useSuccessToast();

  const [showCreateForm, setShowCreateForm] = useState(false);
  const [expiryDays, setExpiryDays] = useState('7');
  const [maxUses, setMaxUses] = useState('10');
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

    try {
      await createInvite(request);
      showSuccess('Invite created', 'The invite link is ready to share.');
      setShowCreateForm(false);
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to create invite';
      showError('Creation failed', message);
    }
  };

  const handleCopyLink = async (code: string, inviteId: string) => {
    const link = `${window.location.origin}/groups/join/${encodeURIComponent(code)}`;
    try {
      await navigator.clipboard.writeText(link);
      setCopiedId(inviteId);
      showSuccess('Invite link copied!', 'Share this link with people you want to invite.');
      setTimeout(() => setCopiedId(null), 2000);
    } catch {
      // Fallback: copy just the code
      try {
        await navigator.clipboard.writeText(code);
        setCopiedId(inviteId);
        showSuccess('Invite code copied!', 'Share this code with people you want to invite.');
        setTimeout(() => setCopiedId(null), 2000);
      } catch {
        showError('Copy failed', 'Could not copy to clipboard.');
      }
    }
  };

  const handleDeactivate = async () => {
    if (!confirmDeactivate) return;
    try {
      await deactivateInvite(confirmDeactivate.id);
      showSuccess('Invite deactivated', 'The invite link will no longer work.');
      setConfirmDeactivate(null);
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to deactivate invite';
      showError('Deactivation failed', message);
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
          <p className="text-sm text-zinc-400">
            Share invite links to let people join this group.
          </p>
          <Button variant="primary" size="sm" onClick={() => setShowCreateForm(!showCreateForm)}>
            <span className="flex items-center gap-2">
              <Plus className="w-4 h-4" />
              New Invite
            </span>
          </Button>
        </div>
      )}

      {/* Create invite form */}
      {showCreateForm && canManage && (
        <Card variant="dark" className="!p-5">
          <h4 className="text-sm font-semibold text-white mb-4">Create Invite Link</h4>
          <div className="grid grid-cols-2 gap-4 mb-4">
            <Select
              label="Expires After"
              options={EXPIRY_OPTIONS}
              value={expiryDays}
              onChange={(e) => setExpiryDays(e.target.value)}
            />
            <Select
              label="Max Uses"
              options={MAX_USES_OPTIONS}
              value={maxUses}
              onChange={(e) => setMaxUses(e.target.value)}
            />
          </div>
          <div className="flex justify-end gap-2">
            <Button variant="secondary" size="sm" onClick={() => setShowCreateForm(false)}>
              Cancel
            </Button>
            <Button variant="primary" size="sm" onClick={handleCreate} loading={isCreating}>
              Create
            </Button>
          </div>
        </Card>
      )}

      {/* Active invites */}
      {activeInvites.length > 0 && (
        <div>
          <h4 className="text-sm font-semibold text-zinc-400 mb-3">
            Active Invites ({activeInvites.length})
          </h4>
          <div className="space-y-2">
            {activeInvites.map((invite) => (
              <div
                key={invite.id}
                className="flex items-center justify-between p-4 rounded-lg bg-white/5 hover:bg-white/10 transition-colors"
              >
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2 mb-1">
                    <Link2 className="w-4 h-4 text-pierre-violet-light flex-shrink-0" />
                    <code className="text-sm text-white font-mono truncate">{invite.code}</code>
                  </div>
                  <div className="flex items-center gap-3 text-xs text-zinc-500">
                    <span>
                      {invite.use_count} / {invite.max_uses ?? 'unlimited'} uses
                    </span>
                    {invite.expires_at && (
                      <span>Expires {formatDate(invite.expires_at)}</span>
                    )}
                    {!invite.expires_at && <span>No expiry</span>}
                  </div>
                </div>
                <div className="flex items-center gap-2 ml-4">
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={() => handleCopyLink(invite.code, invite.id)}
                    title="Copy invite link"
                    aria-label="Copy invite link to clipboard"
                  >
                    {copiedId === invite.id ? (
                      <Check className="w-4 h-4 text-pierre-activity" />
                    ) : (
                      <Copy className="w-4 h-4" />
                    )}
                  </Button>
                  {canManage && (
                    <Button
                      variant="danger"
                      size="sm"
                      onClick={() => setConfirmDeactivate(invite)}
                      title="Deactivate invite"
                      aria-label="Deactivate this invite"
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
          <h4 className="text-sm font-semibold text-zinc-400 mb-3">
            Expired / Used ({inactiveInvites.length})
          </h4>
          <div className="space-y-2">
            {inactiveInvites.map((invite) => (
              <div
                key={invite.id}
                className="flex items-center justify-between p-4 rounded-lg bg-white/5 opacity-50"
              >
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2 mb-1">
                    <Link2 className="w-4 h-4 text-zinc-500 flex-shrink-0" />
                    <code className="text-sm text-zinc-400 font-mono truncate">{invite.code}</code>
                  </div>
                  <div className="flex items-center gap-3 text-xs text-zinc-600">
                    <span>{invite.use_count} uses</span>
                    {!invite.is_active && <span>Deactivated</span>}
                    {invite.is_active && isExpired(invite) && <span>Expired</span>}
                    {invite.is_active && isExhausted(invite) && <span>Max uses reached</span>}
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
          <Link2 className="w-8 h-8 text-zinc-600 mx-auto mb-3" />
          <p className="text-zinc-500">No invites created yet.</p>
          {canManage && (
            <p className="text-zinc-600 text-sm mt-1">
              Create an invite link to let people join this group.
            </p>
          )}
        </div>
      )}

      {/* Confirm deactivation dialog */}
      <ConfirmDialog
        isOpen={!!confirmDeactivate}
        onClose={() => setConfirmDeactivate(null)}
        onConfirm={handleDeactivate}
        title="Deactivate Invite"
        message="This invite link will stop working immediately. Anyone who hasn't used it yet will need a new invite."
        confirmLabel="Deactivate"
        variant="warning"
        isLoading={isDeactivating}
      />
    </div>
  );
}
