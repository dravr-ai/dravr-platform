// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The chat "+" — a menu offering a new chat, a new group chat, and adding someone to the open thread
// ABOUTME: Telegram-shaped: one affordance for starting or widening a conversation, nothing else

import { useEffect, useRef, useState } from 'react';
import { clsx } from 'clsx';
import { MessageSquarePlus, Plus, UserPlus, Users } from 'lucide-react';
import { COMMAND_DRAFTS } from '@pierre/shared-constants';
import { Button, Input, Modal, ModalActions } from '../ui';
import { useTranslation } from '@pierre/i18n';

export interface ChatComposeMenuProps {
  /** Start a fresh one-to-one conversation. */
  onNewChat: () => void;
  /**
   * Start a fresh conversation that sends this `/group create <name>` command.
   * The command creates the group and binds the thread to it, so there is one
   * implementation of "new group chat" across web, mobile and messaging.
   */
  onNewGroupChat: (command: string) => void;
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
 * Coaches are invited with `/coach add @handle` from the composer's slash
 * palette, so the menu names people and rooms only: a new chat, a new group
 * chat — which asks for the group's name and then issues `/group create` —
 * and, once a thread is open, adding someone to it.
 */
export default function ChatComposeMenu({
  onNewChat,
  onNewGroupChat,
  onAddParticipant,
  disabled = false,
}: ChatComposeMenuProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [namingGroup, setNamingGroup] = useState(false);
  const [groupName, setGroupName] = useState('');
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

  const closeNaming = () => {
    setNamingGroup(false);
    setGroupName('');
  };

  const submitGroupName = () => {
    const trimmed = groupName.trim();
    if (!trimmed) return;
    closeNaming();
    onNewGroupChat(COMMAND_DRAFTS.groupCreate(trimmed));
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
        aria-label={t('chat.newMenuButton')}
        title={t('chat.newMenuButton')}
        className="rounded-lg text-on-primary bg-primary hover:bg-primary-container transition-colors shadow-ambient disabled:opacity-50 min-w-[44px] min-h-[44px] flex items-center justify-center"
      >
        <Plus className="w-4 h-4" aria-hidden="true" />
      </button>

      {open && (
        <div
          role="menu"
          aria-label={t('chat.startConversationPrompt')}
          data-testid="chat-compose-menu"
          className="absolute right-0 z-30 mt-2 w-72 max-w-[90vw] rounded-xl border ghost-border bg-surface shadow-lg p-1.5"
        >
          <button type="button" role="menuitem" onClick={() => choose(onNewChat)} className={itemClass}>
            <MessageSquarePlus className="w-4 h-4 text-primary flex-shrink-0" aria-hidden="true" />
            <span>{t('chat.newChat')}</span>
          </button>
          <button
            type="button"
            role="menuitem"
            onClick={() => choose(() => setNamingGroup(true))}
            className={itemClass}
          >
            <Users className="w-4 h-4 text-primary flex-shrink-0" aria-hidden="true" />
            <span>{t('chat.newGroupChat')}</span>
          </button>
          {onAddParticipant && (
            <button
              type="button"
              role="menuitem"
              onClick={() => choose(onAddParticipant)}
              className={clsx(itemClass, 'border-t ghost-border rounded-t-none mt-1 pt-3')}
            >
              <UserPlus className="w-4 h-4 text-primary flex-shrink-0" aria-hidden="true" />
              <span>{t('chat.addSomeoneHint')}</span>
            </button>
          )}
        </div>
      )}

      <Modal
        isOpen={namingGroup}
        onClose={closeNaming}
        title={t('chat.newGroupChat')}
        size="sm"
        footer={
          <ModalActions>
            <Button variant="secondary" onClick={closeNaming}>
              {t('chat.cancel')}
            </Button>
            <Button
              variant="primary"
              onClick={submitGroupName}
              disabled={!groupName.trim()}
              data-testid="group-name-submit"
            >
              {t('groups.inviteCreate')}
            </Button>
          </ModalActions>
        }
      >
        <Input
          label={t('chat.groupNameLabel')}
          value={groupName}
          onChange={(e) => setGroupName(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              e.preventDefault();
              submitGroupName();
            }
          }}
          placeholder={t('groups.namePlaceholder')}
          maxLength={100}
          autoFocus
          data-testid="group-name-input"
        />
      </Modal>
    </div>
  );
}
