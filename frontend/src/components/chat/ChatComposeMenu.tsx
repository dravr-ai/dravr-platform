// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The chat "+" — a menu offering a new chat, a new group chat, and adding someone to the open thread
// ABOUTME: Telegram-shaped: one affordance for starting or widening a conversation, nothing else

import { useEffect, useRef, useState } from 'react';
import { clsx } from 'clsx';
import { MessageSquarePlus, Plus, UserPlus, Users } from 'lucide-react';

export interface ChatComposeMenuProps {
  /** Start a fresh one-to-one conversation. */
  onNewChat: () => void;
  /** Open the group picker for a group-scoped conversation. */
  onNewGroupChat: () => void;
  /**
   * Open the participants control of the open conversation. Undefined when no
   * conversation is open, in which case the item is not offered at all — there
   * is no discussion to add anyone to.
   */
  onAddParticipant?: () => void;
  /** Disables the trigger while a conversation is being created. */
  disabled?: boolean;
}

/**
 * The chat "+" as a menu rather than a bare "new chat" button.
 *
 * Coaches are invited with `/coach invite @handle` from the composer's slash
 * palette, so the menu names people and rooms only: a new chat, a new chat
 * inside one of the athlete's coaching groups, and — once a thread is open —
 * adding someone to it through the conversation's participants control.
 */
export default function ChatComposeMenu({
  onNewChat,
  onNewGroupChat,
  onAddParticipant,
  disabled = false,
}: ChatComposeMenuProps) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(event.target as Node)) {
        setOpen(false);
      }
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setOpen(false);
    };
    document.addEventListener('mousedown', onPointerDown);
    document.addEventListener('keydown', onKeyDown);
    return () => {
      document.removeEventListener('mousedown', onPointerDown);
      document.removeEventListener('keydown', onKeyDown);
    };
  }, [open]);

  const choose = (action: () => void) => {
    setOpen(false);
    action();
  };

  const itemClass =
    'w-full flex items-center gap-3 px-3 py-2.5 text-left text-sm text-on-surface hover:bg-surface-container-low rounded-lg transition-colors min-h-[44px]';

  return (
    <div ref={rootRef} className="relative flex-shrink-0">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        disabled={disabled}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label="New"
        title="New"
        className="rounded-lg text-on-primary bg-primary hover:bg-primary-container transition-colors shadow-ambient disabled:opacity-50 min-w-[44px] min-h-[44px] flex items-center justify-center"
      >
        <Plus className="w-4 h-4" aria-hidden="true" />
      </button>

      {open && (
        <div
          role="menu"
          aria-label="Start a conversation"
          data-testid="chat-compose-menu"
          className="absolute right-0 z-30 mt-2 w-72 max-w-[90vw] rounded-xl border ghost-border bg-surface shadow-lg p-1.5"
        >
          <button type="button" role="menuitem" onClick={() => choose(onNewChat)} className={itemClass}>
            <MessageSquarePlus className="w-4 h-4 text-primary flex-shrink-0" aria-hidden="true" />
            <span>New chat</span>
          </button>
          <button type="button" role="menuitem" onClick={() => choose(onNewGroupChat)} className={itemClass}>
            <Users className="w-4 h-4 text-primary flex-shrink-0" aria-hidden="true" />
            <span>New group chat</span>
          </button>
          {onAddParticipant && (
            <button
              type="button"
              role="menuitem"
              onClick={() => choose(onAddParticipant)}
              className={clsx(itemClass, 'border-t ghost-border rounded-t-none mt-1 pt-3')}
            >
              <UserPlus className="w-4 h-4 text-primary flex-shrink-0" aria-hidden="true" />
              <span>Add someone to this discussion</span>
            </button>
          )}
        </div>
      )}
    </div>
  );
}
