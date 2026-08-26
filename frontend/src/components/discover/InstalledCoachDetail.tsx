// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Detail sheet for one of the athlete's own coaches — sections, stats, handle, and the actions on it
// ABOUTME: Opened from the pinned "Your coaches" grid on Discover; edit and removal flow back to the grid

import { clsx } from 'clsx';
import { MessageCircle, Pencil, Trash2 } from 'lucide-react';
import type { Coach } from '@pierre/shared-types';
import { Button, Modal, ModalActions } from '../ui';
import { categoryBadgeClass, contextPercentage } from './coachCategory';

export interface InstalledCoachDetailProps {
  coach: Coach | null;
  onClose: () => void;
  /** Open a conversation with this coach. */
  onChat: (coach: Coach) => void;
  /** Open the coach editor. Personal coaches only — a system coach is forked instead. */
  onEdit: (coach: Coach) => void;
  /** Remove the coach from the athlete's list (uninstall a copy, delete a personal coach). */
  onRemove: (coach: Coach) => void;
  onToggleFavorite: (coach: Coach) => void;
  /** Error from the last action on this coach, shown inside the sheet. */
  actionError: string | null;
  isRemoving: boolean;
}

interface SectionProps {
  title: string;
  body: string;
  mono?: boolean;
}

function Section({ title, body, mono = false }: SectionProps) {
  return (
    <div>
      <h3 className="text-sm font-medium text-on-surface mb-2">{title}</h3>
      <div
        className={clsx(
          'p-4 bg-surface-container-low rounded-lg text-sm text-on-surface whitespace-pre-wrap',
          mono && 'font-mono max-h-48 overflow-y-auto',
        )}
      >
        {body}
      </div>
    </div>
  );
}

export default function InstalledCoachDetail({
  coach,
  onClose,
  onChat,
  onEdit,
  onRemove,
  onToggleFavorite,
  actionError,
  isRemoving,
}: InstalledCoachDetailProps) {
  if (!coach) return null;
  const structured = Boolean(coach.purpose || coach.instructions);

  return (
    <Modal
      isOpen
      onClose={onClose}
      title={coach.title}
      size="2xl"
      footer={
        <ModalActions className="flex-wrap">
          {!coach.is_system && (
            <Button variant="danger" onClick={() => onRemove(coach)} disabled={isRemoving}>
              <Trash2 className="w-4 h-4 mr-2" aria-hidden="true" />
              {isRemoving ? 'Removing...' : coach.forked_from ? 'Remove' : 'Delete'}
            </Button>
          )}
          {!coach.is_system && (
            <Button variant="secondary" onClick={() => onEdit(coach)}>
              <Pencil className="w-4 h-4 mr-2" aria-hidden="true" />
              Edit
            </Button>
          )}
          <Button onClick={() => onChat(coach)}>
            <MessageCircle className="w-4 h-4 mr-2" aria-hidden="true" />
            Chat
          </Button>
        </ModalActions>
      }
    >
      <div className="space-y-6">
        <div className="flex flex-wrap items-center gap-2">
          {coach.handle && (
            <span className="font-mono text-sm text-primary" data-testid="coach-handle">
              @{coach.handle}
            </span>
          )}
          <span
            className={clsx(
              'px-2 py-0.5 text-xs font-medium rounded-full border capitalize',
              categoryBadgeClass(coach.category),
            )}
          >
            {coach.category}
          </span>
          {coach.is_system && (
            <span className="px-2 py-0.5 text-xs font-medium rounded-full bg-primary/10 text-primary border border-primary/20">
              System
            </span>
          )}
          <button
            type="button"
            onClick={() => onToggleFavorite(coach)}
            className="ml-auto min-w-[44px] min-h-[44px] flex items-center justify-center text-outline hover:text-warning transition-colors"
            title={coach.is_favorite ? 'Remove from favorites' : 'Add to favorites'}
            aria-label={coach.is_favorite ? 'Remove from favorites' : 'Add to favorites'}
          >
            <svg
              className={clsx('w-6 h-6', coach.is_favorite ? 'fill-warning text-warning' : 'fill-none')}
              stroke="currentColor"
              viewBox="0 0 24 24"
              aria-hidden="true"
            >
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M11.049 2.927c.3-.921 1.603-.921 1.902 0l1.519 4.674a1 1 0 00.95.69h4.915c.969 0 1.371 1.24.588 1.81l-3.976 2.888a1 1 0 00-.363 1.118l1.518 4.674c.3.922-.755 1.688-1.538 1.118l-3.976-2.888a1 1 0 00-1.176 0l-3.976 2.888c-.783.57-1.838-.197-1.538-1.118l1.518-4.674a1 1 0 00-.363-1.118l-3.976-2.888c-.784-.57-.38-1.81.588-1.81h4.914a1 1 0 00.951-.69l1.519-4.674z"
              />
            </svg>
          </button>
        </div>

        {actionError && (
          <div className="p-3 rounded-lg bg-error/10 border border-error/30">
            <p className="text-sm text-error">{actionError}</p>
          </div>
        )}

        {coach.description && <p className="text-on-surface-variant">{coach.description}</p>}

        <div className="grid grid-cols-3 gap-4 p-4 bg-surface-container-low rounded-lg">
          <div className="text-center">
            <div className="text-2xl font-bold text-primary">~{coach.token_count.toLocaleString()}</div>
            <div className="text-xs text-on-surface-variant">Tokens ({contextPercentage(coach.token_count)}% context)</div>
          </div>
          <div className="text-center">
            <div className="text-2xl font-bold text-activity">{coach.use_count}</div>
            <div className="text-xs text-on-surface-variant">Uses</div>
          </div>
          <div className="text-center">
            <div className="text-2xl font-bold text-nutrition">{coach.is_favorite ? '★' : '☆'}</div>
            <div className="text-xs text-on-surface-variant">{coach.is_favorite ? 'Favorite' : 'Not Favorite'}</div>
          </div>
        </div>

        {structured ? (
          <div className="space-y-4">
            {coach.purpose && <Section title="Purpose" body={coach.purpose} />}
            {coach.when_to_use && <Section title="When to Use" body={coach.when_to_use} />}
            {coach.instructions && <Section title="Instructions" body={coach.instructions} mono />}
            {coach.example_inputs && <Section title="Example Inputs" body={coach.example_inputs} />}
            {coach.example_outputs && <Section title="Example Outputs" body={coach.example_outputs} />}
            {coach.success_criteria && <Section title="Success Criteria" body={coach.success_criteria} />}
          </div>
        ) : (
          <Section title="System Prompt" body={coach.system_prompt} mono />
        )}

        {coach.tags.length > 0 && (
          <div>
            <h3 className="text-sm font-medium text-on-surface mb-2">Tags</h3>
            <div className="flex flex-wrap gap-2">
              {coach.tags.map((tag) => (
                <span key={tag} className="px-3 py-1 text-sm bg-surface-container-low text-on-surface rounded-full">
                  {tag}
                </span>
              ))}
            </div>
          </div>
        )}

        <div className="grid grid-cols-2 gap-4 text-sm text-on-surface-variant pt-4 border-t ghost-border">
          <div>
            <span className="font-medium">Created:</span> {new Date(coach.created_at).toLocaleString()}
          </div>
          <div>
            <span className="font-medium">Last Updated:</span> {new Date(coach.updated_at).toLocaleString()}
          </div>
        </div>
      </div>
    </Modal>
  );
}
