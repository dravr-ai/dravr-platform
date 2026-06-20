// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Individual conversation item in the sidebar list
// ABOUTME: Memoized for performance when rendering many conversations

import { memo, useRef, useEffect } from 'react';
import { clsx } from 'clsx';
import { History, Pencil, Trash2, Send } from 'lucide-react';
import { resolveChannelOrigin } from '@pierre/chat-utils';
import type { Conversation } from './types';
import { formatDate } from './utils';

interface ConversationItemProps {
  conversation: Conversation;
  /** Title to display for the row. Defaults to the conversation's own title.
   *  ConversationsPanel passes a coach-aware "<Coach> · <time>" form here. */
  displayTitle?: string;
  isSelected: boolean;
  isEditing: boolean;
  editedTitleValue: string;
  onSelect: () => void;
  onStartRename: (e: React.MouseEvent) => void;
  onDelete: (e: React.MouseEvent) => void;
  onTitleChange: (value: string) => void;
  onSaveRename: () => void;
  onCancelRename: () => void;
}

const ConversationItem = memo(function ConversationItem({
  conversation,
  displayTitle,
  isSelected,
  isEditing,
  editedTitleValue,
  onSelect,
  onStartRename,
  onDelete,
  onTitleChange,
  onSaveRename,
  onCancelRename,
}: ConversationItemProps) {
  const titleInputRef = useRef<HTMLInputElement>(null);

  // Origin badge for conversations that started on a messaging channel
  // (Telegram, WhatsApp, …). Prefers the durable channel_type column (survives
  // a rename/reset), falling back to the "Messaging: <channel>" title — never
  // the possibly-rewritten displayTitle. Plain web/mobile chats → null.
  const channelOrigin = resolveChannelOrigin(conversation);

  // Focus input when editing starts
  useEffect(() => {
    if (isEditing && titleInputRef.current) {
      titleInputRef.current.focus();
      titleInputRef.current.select();
    }
  }, [isEditing]);

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter') {
      onSaveRename();
    } else if (e.key === 'Escape') {
      onCancelRename();
    }
  };

  return (
    <button
      onClick={() => {
        if (!isEditing) {
          onSelect();
        }
      }}
      className={clsx(
        'w-full flex items-center gap-3 px-3 py-2 rounded-lg group transition-colors text-left',
        isSelected
          ? 'bg-surface-container-high text-on-surface'
          : 'hover:bg-surface-container-low text-on-surface'
      )}
    >
      <div className="text-outline group-hover:text-on-surface transition-colors flex-shrink-0">
        <History className="w-4 h-4" aria-hidden="true" />
      </div>
      <div className="flex-1 min-w-0">
        {isEditing ? (
          <input
            ref={titleInputRef}
            type="text"
            value={editedTitleValue}
            onChange={(e) => onTitleChange(e.target.value)}
            onKeyDown={handleKeyDown}
            onBlur={onSaveRename}
            className="w-full text-sm font-medium text-on-surface bg-surface-container-low border border-primary rounded px-2 py-0.5 focus:outline-none focus:ring-1 focus:ring-primary"
            onClick={(e) => e.stopPropagation()}
          />
        ) : (
          <div className="flex items-center gap-1.5 min-w-0">
            {channelOrigin && (
              <span
                className="inline-flex items-center gap-1 flex-shrink-0 rounded-full bg-primary/15 text-primary px-1.5 py-0.5 text-[10px] font-medium"
                title={`From ${channelOrigin.label}`}
                aria-label={`From ${channelOrigin.label}`}
                data-testid="conversation-channel-badge"
              >
                <Send className="w-2.5 h-2.5" aria-hidden="true" />
                {channelOrigin.label}
              </span>
            )}
            <p className="text-sm font-normal truncate group-hover:text-on-surface transition-colors">
              {displayTitle ?? conversation.title ?? 'Untitled Chat'}
            </p>
          </div>
        )}
      </div>
      <span className="text-on-surface-variant text-xs whitespace-nowrap flex-shrink-0">
        {formatDate(conversation.updated_at)}
      </span>
      {/* Action buttons on hover */}
      <div className="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity">
        <button
          onClick={onStartRename}
          className="p-2.5 rounded transition-colors text-outline hover:text-primary hover:bg-surface-container"
          title="Rename"
          aria-label="Rename conversation"
        >
          <Pencil className="w-4 h-4" aria-hidden="true" />
        </button>
        <button
          onClick={onDelete}
          className="p-2.5 rounded transition-colors text-outline hover:text-pierre-red-500 hover:bg-surface-container"
          title="Delete"
          aria-label="Delete conversation"
        >
          <Trash2 className="w-4 h-4" aria-hidden="true" />
        </button>
      </div>
    </button>
  );
});

export default ConversationItem;
