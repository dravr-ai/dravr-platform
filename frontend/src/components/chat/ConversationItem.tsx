// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: One row of the unified conversation list — avatar, kind glyph, title, preview, time, unread count
// ABOUTME: Bold while unread; a hover menu holds rename, mark-unread and delete; the rename is an inline ui Input

import { memo, useEffect, useRef, useState } from 'react';
import { clsx } from 'clsx';
import { Mail, MoreVertical, Pencil, Send, Trash2, Users } from 'lucide-react';
import type { ConversationRowModel } from '@pierre/chat-utils';
import { Badge, IconButton, Input } from '../ui';
import { avatarSlotClass } from './avatarSlots';

interface ConversationItemProps {
  row: ConversationRowModel;
  isSelected: boolean;
  isEditing: boolean;
  editedTitleValue: string;
  onSelect: () => void;
  onStartRename: () => void;
  /** Clear the caller's read marker — every row counts as unread again. */
  onMarkUnread: () => void;
  onDelete: () => void;
  onTitleChange: (value: string) => void;
  onSaveRename: () => void;
  onCancelRename: () => void;
}

/**
 * The row's own actions, behind one control at the far right edge.
 *
 * Three buttons laid over the row covered more than half of a 260px sidebar
 * row while the pointer was on it, so the right half of every row stopped
 * selecting the conversation. One trigger leaves the row clickable, which is
 * also how Telegram and iMessage reach the same three actions.
 */
function RowMenu({
  open,
  setOpen,
  onStartRename,
  onMarkUnread,
  onDelete,
}: {
  open: boolean;
  setOpen: (open: boolean) => void;
  onStartRename: () => void;
  onMarkUnread: () => void;
  onDelete: () => void;
}) {
  const rootRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(event.target as Node)) setOpen(false);
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
  }, [open, setOpen]);

  const choose = (action: () => void) => {
    setOpen(false);
    action();
  };

  const itemClass =
    'w-full flex items-center gap-2.5 px-3 py-2 text-left text-sm text-on-surface hover:bg-surface-container-low rounded-lg transition-colors min-h-[44px]';

  return (
    <div
      ref={rootRef}
      className={clsx(
        'absolute right-1 top-1/2 -translate-y-1/2',
        open
          ? 'opacity-100'
          : 'opacity-0 pointer-events-none group-hover:opacity-100 group-hover:pointer-events-auto group-focus-within:opacity-100 group-focus-within:pointer-events-auto',
      )}
    >
      <IconButton
        size="sm"
        variant="tonal"
        onClick={() => setOpen(!open)}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label="Conversation actions"
        title="Conversation actions"
        data-testid="conversation-actions-trigger"
      >
        <MoreVertical className="w-4 h-4" aria-hidden="true" />
      </IconButton>
      {open && (
        <div
          role="menu"
          aria-label="Conversation actions"
          data-testid="conversation-actions-menu"
          className="absolute right-0 z-30 mt-1 w-48 rounded-xl border ghost-border bg-surface shadow-lg p-1.5"
        >
          <button type="button" role="menuitem" onClick={() => choose(onStartRename)} className={itemClass}>
            <Pencil className="w-4 h-4 text-primary flex-shrink-0" aria-hidden="true" />
            <span>Rename conversation</span>
          </button>
          <button type="button" role="menuitem" onClick={() => choose(onMarkUnread)} className={itemClass}>
            <Mail className="w-4 h-4 text-primary flex-shrink-0" aria-hidden="true" />
            <span>Mark conversation unread</span>
          </button>
          <button
            type="button"
            role="menuitem"
            onClick={() => choose(onDelete)}
            className={clsx(itemClass, 'hover:!text-error')}
          >
            <Trash2 className="w-4 h-4 text-error flex-shrink-0" aria-hidden="true" />
            <span>Delete conversation</span>
          </button>
        </div>
      )}
    </div>
  );
}

/** What the glyph before the title says, for the rows that carry one. */
function KindGlyph({ row }: { row: ConversationRowModel }) {
  if (row.kind === 'group') {
    return (
      <span
        className="inline-flex items-center flex-shrink-0 text-on-surface-variant"
        data-testid="conversation-kind-glyph"
        data-kind="group"
        title={row.groupName ?? 'Group chat'}
        aria-label="Group chat"
      >
        <Users className="w-3.5 h-3.5" aria-hidden="true" />
      </span>
    );
  }
  if (row.kind === 'channel' && row.channel) {
    return (
      <span
        className="inline-flex items-center gap-1 flex-shrink-0 rounded-full bg-primary/15 text-primary px-1.5 py-0.5 text-[10px] font-medium"
        title={`From ${row.channel.label}`}
        aria-label={`From ${row.channel.label}`}
        data-testid="conversation-channel-badge"
        data-kind="channel"
      >
        <Send className="w-2.5 h-2.5" aria-hidden="true" />
        {row.channel.label}
      </span>
    );
  }
  return null;
}

const ConversationItem = memo(function ConversationItem({
  row,
  isSelected,
  isEditing,
  editedTitleValue,
  onSelect,
  onStartRename,
  onMarkUnread,
  onDelete,
  onTitleChange,
  onSaveRename,
  onCancelRename,
}: ConversationItemProps) {
  const unread = row.unreadCount > 0;
  // The open menu drops over the rows below it, so the row it belongs to has
  // to sit above its later siblings while it is showing — a `relative` row at
  // `z-index: auto` is painted in DOM order, which put the next row on top.
  const [menuOpen, setMenuOpen] = useState(false);

  const avatar = (
    <div
      className={clsx(
        'w-10 h-10 rounded-full flex items-center justify-center flex-shrink-0 text-sm font-semibold select-none',
        avatarSlotClass(row.avatarSlot),
      )}
      data-testid="conversation-avatar"
      aria-hidden="true"
    >
      {row.initials}
    </div>
  );

  if (isEditing) {
    return (
      <li
        className="flex items-center gap-3 px-3 py-2 rounded-lg bg-surface-container-high"
        data-testid="conversation-row"
        data-conversation-id={row.id}
      >
        {avatar}
        <div className="flex-1 min-w-0">
          <Input
            type="text"
            size="sm"
            value={editedTitleValue}
            onChange={(e) => onTitleChange(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault();
                onSaveRename();
              } else if (e.key === 'Escape') {
                e.preventDefault();
                onCancelRename();
              }
            }}
            onBlur={onSaveRename}
            aria-label="Conversation title"
            autoFocus
          />
        </div>
      </li>
    );
  }

  return (
    <li
      className={clsx('group relative', menuOpen && 'z-20')}
      data-testid="conversation-row"
      data-conversation-id={row.id}
      data-unread={unread ? 'true' : undefined}
    >
      <button
        type="button"
        onClick={onSelect}
        aria-current={isSelected ? 'true' : undefined}
        className={clsx(
          'w-full flex items-center gap-3 px-3 py-2 rounded-lg text-left transition-colors min-h-[56px]',
          isSelected ? 'bg-surface-container-high' : 'hover:bg-surface-container-low',
        )}
      >
        {avatar}
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-1.5 min-w-0">
            <KindGlyph row={row} />
            <p
              className={clsx(
                'text-sm truncate text-on-surface',
                unread ? 'font-semibold' : 'font-normal',
              )}
              data-testid="conversation-title"
            >
              {row.title}
            </p>
          </div>
          <p
            className={clsx(
              'text-xs truncate',
              unread ? 'text-on-surface-variant' : 'text-outline',
            )}
            data-testid="conversation-preview"
          >
            {row.preview || (row.coachHandle ? `@${row.coachHandle}` : ' ')}
          </p>
        </div>
        <div className="flex flex-col items-end gap-1 flex-shrink-0 min-w-[2.5rem]">
          <span
            className={clsx('text-[11px] whitespace-nowrap', unread ? 'text-primary' : 'text-outline')}
            data-testid="conversation-timestamp"
          >
            {row.timestamp}
          </span>
          {unread && (
            <Badge
              variant="info"
              className="!px-1.5 !py-0 !text-[10px] min-w-[1.25rem] h-5 justify-center"
            >
              <span data-testid="conversation-unread-count" aria-label={`${row.unreadCount} unread`}>
                {row.unreadCount > 99 ? '99+' : row.unreadCount}
              </span>
            </Badge>
          )}
        </div>
      </button>
      <RowMenu
        open={menuOpen}
        setOpen={setMenuOpen}
        onStartRename={onStartRename}
        onMarkUnread={onMarkUnread}
        onDelete={onDelete}
      />
    </li>
  );
});

export default ConversationItem;
