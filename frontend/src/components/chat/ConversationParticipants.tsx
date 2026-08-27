// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Who is in the open conversation — list, add by user id, remove a member
// ABOUTME: The web caller of the participants routes; the owner is listed but never removable

import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Users, UserPlus, X } from 'lucide-react';
import { chatApi } from '../../services/api';
import { QUERY_KEYS } from '../../constants/queryKeys';
import { Input, useErrorToast, useSuccessToast } from '../ui';
import type { ConversationParticipant } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';

interface ConversationParticipantsProps {
  conversationId: string;
  /**
   * Controlled open state. The chat "+" menu opens this control from outside
   * ("Add someone to this discussion"); left undefined, the toggle owns it.
   */
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
}

function participantLabel(p: ConversationParticipant): string {
  return p.role === 'owner' ? `${p.user_id} · owner` : p.user_id;
}

/**
 * A compact "Participants (N)" toggle for the conversation header. Expanded,
 * it lists everyone in the thread and lets any participant add a tenant
 * member by user id or remove a member. Refusals come back from the server
 * (a non-member of the tenant is 403, the owner cannot be removed) and are
 * surfaced as toasts rather than guessed at client-side.
 */
export default function ConversationParticipants({
  conversationId,
  open: controlledOpen,
  onOpenChange,
}: ConversationParticipantsProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const errorToast = useErrorToast();
  const successToast = useSuccessToast();
  const [uncontrolledOpen, setUncontrolledOpen] = useState(false);
  const open = controlledOpen ?? uncontrolledOpen;
  const setOpen = (next: boolean) => {
    onOpenChange?.(next);
    if (controlledOpen === undefined) setUncontrolledOpen(next);
  };
  const [newUserId, setNewUserId] = useState('');

  const { data: participants = [], isLoading } = useQuery({
    queryKey: QUERY_KEYS.chat.participants(conversationId),
    queryFn: () => chatApi.listParticipants(conversationId),
  });

  const invalidate = () =>
    queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.participants(conversationId) });

  const addParticipant = useMutation({
    mutationFn: (userId: string) => chatApi.addParticipant(conversationId, userId),
    onSuccess: async () => {
      setNewUserId('');
      await invalidate();
      successToast('Participant added');
    },
    onError: (error: unknown) => {
      errorToast(error instanceof Error ? error.message : t('chat.addParticipantFailed'));
    },
  });

  const removeParticipant = useMutation({
    mutationFn: (userId: string) => chatApi.removeParticipant(conversationId, userId),
    onSuccess: async () => {
      await invalidate();
      successToast('Participant removed');
    },
    onError: (error: unknown) => {
      errorToast(error instanceof Error ? error.message : t('chat.removeParticipantFailed'));
    },
  });

  const submitAdd = (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const trimmed = newUserId.trim();
    if (!trimmed) return;
    addParticipant.mutate(trimmed);
  };

  return (
    <div className="relative flex-shrink-0">
      <button
        type="button"
        onClick={() => setOpen(!open)}
        aria-expanded={open}
        className="inline-flex items-center gap-1.5 px-2.5 py-1.5 text-xs font-medium text-on-surface-variant hover:text-on-surface hover:bg-surface-container-low rounded-lg transition-colors"
        title={t('chat.participantsHeading')}
      >
        <Users className="w-3.5 h-3.5" aria-hidden="true" />
        <span>{t('frag.participants')}{isLoading ? '' : ` (${participants.length})`}</span>
      </button>

      {open && (
        <div
          role="dialog"
          aria-label={t('chat.participantsAria')}
          className="absolute right-0 z-20 mt-2 w-80 max-w-[90vw] rounded-xl border ghost-border bg-surface shadow-lg p-3"
        >
          <ul className="space-y-1 max-h-60 overflow-y-auto" aria-label={t('chat.participantListAria')}>
            {participants.map(p => (
              <li
                key={p.user_id}
                className="flex items-center justify-between gap-2 text-xs text-on-surface font-mono"
              >
                <span className="truncate" title={p.user_id}>
                  {participantLabel(p)}
                </span>
                {p.role !== 'owner' && (
                  <button
                    type="button"
                    onClick={() => removeParticipant.mutate(p.user_id)}
                    disabled={removeParticipant.isPending}
                    aria-label={`Remove ${p.user_id}`}
                    className="p-1 rounded text-on-surface-variant hover:text-error hover:bg-error/10 disabled:opacity-50"
                  >
                    <X className="w-3.5 h-3.5" aria-hidden="true" />
                  </button>
                )}
              </li>
            ))}
          </ul>

          <form onSubmit={submitAdd} className="mt-3 flex items-center gap-2">
            <div className="flex-1 min-w-0">
              <Input
                type="text"
                size="sm"
                value={newUserId}
                onChange={e => setNewUserId(e.target.value)}
                placeholder={t('chat.participantIdInput')}
                aria-label={t('chat.participantIdInput')}
              />
            </div>
            <button
              type="submit"
              disabled={addParticipant.isPending || newUserId.trim() === ''}
              className="inline-flex items-center gap-1 px-2 py-1 text-xs font-medium text-primary bg-primary/10 hover:bg-primary/20 rounded-lg disabled:opacity-50 disabled:cursor-not-allowed"
            >
              <UserPlus className="w-3.5 h-3.5" aria-hidden="true" />
              {t('chat.addParticipant')}
            </button>
          </form>
        </div>
      )}
    </div>
  );
}
