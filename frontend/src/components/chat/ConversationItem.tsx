// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Individual conversation item in the sidebar list
// ABOUTME: Memoized for performance when rendering many conversations

import { memo, useRef, useEffect } from 'react';
import { clsx } from 'clsx';
import { History, Pencil, Trash2 } from 'lucide-react';
import type { Conversation } from './types';
import { formatDate } from './utils';

interface ConversationItemProps {
  conversation: Conversation;
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
            className="w-full text-sm font-medium text-on-surface bg-surface-container-low border border-pierre-violet rounded px-2 py-0.5 focus:outline-none focus:ring-1 focus:ring-pierre-violet"
            onClick={(e) => e.stopPropagation()}
          />
        ) : (
          <p className="text-sm font-normal truncate group-hover:text-on-surface transition-colors">
            {conversation.title ?? 'Untitled Chat'}
          </p>
        )}
      </div>
      <span className="text-on-surface-variant text-xs whitespace-nowrap flex-shrink-0">
        {formatDate(conversation.updated_at)}
      </span>
      {/* Action buttons on hover */}
      <div className="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity">
        <button
          onClick={onStartRename}
          className="p-2.5 rounded transition-colors text-outline hover:text-pierre-violet hover:bg-surface-container"
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
