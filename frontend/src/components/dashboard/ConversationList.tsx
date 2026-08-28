// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The unified conversation list — a search pinned above flat rows sorted by last activity
// ABOUTME: One list for every thread the athlete is in, whatever surface created it; Telegram-shaped

import { useState } from 'react';
import { Search } from 'lucide-react';
import type { ConversationRowModel } from '@pierre/chat-utils';
import { useConversationList, useConversationMutations } from '../../hooks/useConversationList';
import ConversationItem from '../chat/ConversationItem';
import { Button, ConfirmDialog, Input } from '../ui';
import { useTranslation } from '@pierre/i18n';

interface ConversationListProps {
  selectedConversation: string | null;
  onSelectConversation: (id: string | null) => void;
}

/**
 * Every conversation the athlete takes part in, as one flat list.
 *
 * Rows are the shared row model, so a Telegram DM, a coach thread and a group
 * room sit in the same order and carry the same anatomy here as on mobile.
 * Selecting a row only opens it; the read marker is the thread's business
 * and moves once its messages resolve.
 */
export default function ConversationList({
  selectedConversation,
  onSelectConversation,
}: ConversationListProps) {
  const { t } = useTranslation();
  const [searchQuery, setSearchQuery] = useState('');
  const [editingConversationId, setEditingConversationId] = useState<string | null>(null);
  const [editedTitleValue, setEditedTitleValue] = useState('');
  const [deleteConfirmation, setDeleteConfirmation] = useState<ConversationRowModel | null>(null);

  const { rows, isLoading, isError, hasMore, isLoadingMore, loadMore, refetch } =
    useConversationList(searchQuery);
  const { rename, remove, isRemoving, markUnread } = useConversationMutations();

  const searchActive = searchQuery.trim().length > 0;

  const handleStartRename = (row: ConversationRowModel): void => {
    setEditingConversationId(row.id);
    setEditedTitleValue(row.title);
  };

  const handleSaveRename = (): void => {
    const trimmed = editedTitleValue.trim();
    if (editingConversationId && trimmed) {
      void rename(editingConversationId, trimmed);
    }
    setEditingConversationId(null);
    setEditedTitleValue('');
  };

  const handleCancelRename = (): void => {
    setEditingConversationId(null);
    setEditedTitleValue('');
  };

  const handleConfirmDelete = async (): Promise<void> => {
    if (!deleteConfirmation) return;
    const { id } = deleteConfirmation;
    await remove(id);
    if (selectedConversation === id) {
      onSelectConversation(null);
    }
    setDeleteConfirmation(null);
  };

  return (
    <div className="flex flex-col h-full min-h-0" data-testid="conversation-list">
      <div className="px-3 pt-3 pb-2 flex-shrink-0">
        <Input
          type="search"
          size="sm"
          leftIcon={<Search className="w-3.5 h-3.5" />}
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
          placeholder={t('convPanel.searchChats')}
          aria-label={t('convPanel.search')}
        />
      </div>

      <div className="flex-1 min-h-0 overflow-y-auto px-2 pb-2">
        {isLoading ? (
          <p className="px-3 py-2 text-outline text-sm">{t('chat.conversationsLoading')}</p>
        ) : isError ? (
          <div className="px-3 py-2 space-y-2">
            <p className="text-sm text-error">{t('chat.listLoadFailed')}</p>
            <Button variant="secondary" size="sm" onClick={refetch}>
              {t('chat.listRetry')}
            </Button>
          </div>
        ) : rows.length === 0 ? (
          <p className="px-3 py-6 text-center text-sm text-outline" data-testid="conversation-list-empty">
            {searchActive ? t('convPanel.noChatsMatch') : t('chat.noChatsEmptyHint')}
          </p>
        ) : (
          <>
            <ul className="space-y-0.5" aria-label={t('chat.conversations')}>
              {rows.map((row) => (
                <ConversationItem
                  key={row.id}
                  row={row}
                  isSelected={selectedConversation === row.id}
                  isEditing={editingConversationId === row.id}
                  editedTitleValue={editedTitleValue}
                  onSelect={() => onSelectConversation(row.id)}
                  onStartRename={() => handleStartRename(row)}
                  onMarkUnread={() => void markUnread(row.id)}
                  onDelete={() => setDeleteConfirmation(row)}
                  onTitleChange={setEditedTitleValue}
                  onSaveRename={handleSaveRename}
                  onCancelRename={handleCancelRename}
                />
              ))}
            </ul>
            {hasMore && !searchActive && (
              <div className="px-3 py-3 flex justify-center">
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={loadMore}
                  loading={isLoadingMore}
                  data-testid="conversation-list-load-more"
                >
                  {t('chat.loadMore')}
                </Button>
              </div>
            )}
          </>
        )}
      </div>

      <ConfirmDialog
        isOpen={!!deleteConfirmation}
        title={t('convPanel.deleteOne')}
        message={`Are you sure you want to delete "${deleteConfirmation?.title ?? ''}"? This action cannot be undone.`}
        confirmLabel="Delete"
        cancelLabel={t('common.cancel')}
        onConfirm={() => void handleConfirmDelete()}
        onClose={() => setDeleteConfirmation(null)}
        variant="danger"
        isLoading={isRemoving}
      />
    </div>
  );
}
