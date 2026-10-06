// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A conversation list — its title and "+", a quiet search, text-tab filters, then flat rows sorted by last activity
// ABOUTME: Scoped to one side of the app: the Groups tab's rooms, or the athlete's own threads in Home's history

import { useMemo, useState, type ReactNode } from 'react';
import { clsx } from 'clsx';
import { scopeOfKind, type ConversationRowModel, type ConversationScope } from '@pierre/chat-utils';
import { useConversationList, useConversationMutations } from '../../hooks/useConversationList';
import ConversationItem from '../chat/ConversationItem';
import { Button, ConfirmDialog, SearchField } from '../ui';
import { useTranslation } from '@pierre/i18n';

interface ConversationListProps {
  /** Which side of the app the list draws: the Groups tab's rooms, or the athlete's own threads. */
  scope: ConversationScope;
  selectedConversation: string | null;
  onSelectConversation: (id: string | null) => void;
  /** The chat "+" menu, rendered by the host so it stays wired to the one conversation-creating mutation. */
  compose?: ReactNode;
  /**
   * Whether the list draws its own title row. Off where a host already names
   * it — the history sheet, whose header holds the title and the "+".
   */
  showTitle?: boolean;
}

/** Which rows the chips above the list keep. */
type RowFilter = 'all' | 'unread' | 'coaches';

const FILTERS_BY_SCOPE: Record<ConversationScope, { key: RowFilter; labelKey: string }[]> = {
  groups: [
    { key: 'all', labelKey: 'discover.filterAll' },
    { key: 'unread', labelKey: 'chat.filterUnread' },
  ],
  personal: [
    { key: 'all', labelKey: 'discover.filterAll' },
    { key: 'unread', labelKey: 'chat.filterUnread' },
    { key: 'coaches', labelKey: 'chat.filterAgents' },
  ],
};

const TEXT_BY_SCOPE: Record<ConversationScope, { title: string; search: string; empty: string }> = {
  groups: { title: 'nav.groups', search: 'groups.searchPlaceholder', empty: 'groups.emptyTitle' },
  personal: {
    title: 'home.personal.history',
    search: 'home.personal.historySearch',
    empty: 'home.personal.historyEmpty',
  },
};

function keepsRow(filter: RowFilter, row: ConversationRowModel): boolean {
  switch (filter) {
    case 'unread':
      return row.unreadCount > 0;
    case 'coaches':
      return row.kind === 'coach';
    case 'all':
    default:
      return true;
  }
}

/**
 * One side of the athlete's conversations, as one flat list.
 *
 * Rows are the shared row model, so a Telegram DM and an agent thread sit in
 * the same order and carry the same anatomy here as on mobile; a room is the
 * Groups tab's, never Home's, and the scope decides which of the two a list
 * keeps. Selecting a row only opens it; the read marker is the thread's
 * business and moves once its messages resolve.
 */
export default function ConversationList({
  scope,
  selectedConversation,
  onSelectConversation,
  compose,
  showTitle = true,
}: ConversationListProps) {
  const { t } = useTranslation();
  const [searchQuery, setSearchQuery] = useState('');
  const [filter, setFilter] = useState<RowFilter>('all');
  const [editingConversationId, setEditingConversationId] = useState<string | null>(null);
  const [editedTitleValue, setEditedTitleValue] = useState('');
  const [deleteConfirmation, setDeleteConfirmation] = useState<ConversationRowModel | null>(null);

  const { rows, isLoading, isError, hasMore, isLoadingMore, loadMore, refetch } =
    useConversationList(searchQuery);
  const { rename, remove, isRemoving, markUnread } = useConversationMutations();

  const searchActive = searchQuery.trim().length > 0;
  const visibleRows = useMemo(
    () => rows.filter((row) => scopeOfKind(row.kind) === scope && keepsRow(filter, row)),
    [rows, filter, scope],
  );
  const text = TEXT_BY_SCOPE[scope];
  // The scope is applied to the loaded pages, so a page can hold none of this
  // side's rows while later pages do: the list then offers the next page
  // rather than saying the side is empty.
  const canLoadMore = hasMore && !searchActive;

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
    <div className="flex h-full min-h-0 flex-col" data-testid="conversation-list" data-scope={scope}>
      <div className="flex-shrink-0 px-4">
        {showTitle && (
          <div className="flex h-[52px] items-center justify-between gap-2">
            <h2 className="font-display text-xl font-semibold text-on-surface">{t(text.title)}</h2>
            {compose ? <div className="flex items-center">{compose}</div> : null}
          </div>
        )}
        <SearchField
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
          placeholder={t(text.search)}
          aria-label={t(text.search)}
        />
        <div className="mt-2.5 flex gap-[18px] border-b ghost-border" role="radiogroup" aria-label={t(text.title)}>
          {FILTERS_BY_SCOPE[scope].map((entry) => {
            const active = filter === entry.key;
            return (
              <button
                key={entry.key}
                type="button"
                role="radio"
                aria-checked={active}
                data-testid={`conversation-filter-${entry.key}`}
                onClick={() => setFilter(entry.key)}
                className={clsx(
                  '-mb-px flex touch-target justify-center border-b-2 pb-2.5 text-sm font-medium transition-colors focus-ring',
                  active
                    ? 'border-primary text-on-surface'
                    : 'border-transparent text-on-surface-variant hover:text-on-surface',
                )}
              >
                {t(entry.labelKey)}
              </button>
            );
          })}
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto">
        {isLoading ? (
          <p className="px-4 py-3 text-sm text-outline">{t('chat.conversationsLoading')}</p>
        ) : isError ? (
          <div className="space-y-2 px-4 py-3">
            <p className="text-sm text-error">{t('chat.listLoadFailed')}</p>
            <Button variant="secondary" size="sm" onClick={refetch}>
              {t('chat.listRetry')}
            </Button>
          </div>
        ) : visibleRows.length === 0 && !canLoadMore ? (
          <p className="px-4 py-8 text-center text-sm text-outline" data-testid="conversation-list-empty">
            {searchActive || filter !== 'all' ? t('convPanel.noChatsMatch') : t(text.empty)}
          </p>
        ) : (
          <>
            {visibleRows.length > 0 && (
              <ul aria-label={t('chat.conversations')}>
                {visibleRows.map((row) => (
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
            )}
            {canLoadMore && (
              <div className="flex justify-center px-3 py-3">
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
        message={t('app.confirmDeleteConversation', { title: deleteConfirmation?.title ?? '' })}
        confirmLabel={t('common.delete')}
        cancelLabel={t('common.cancel')}
        onConfirm={() => void handleConfirmDelete()}
        onClose={() => setDeleteConfirmation(null)}
        variant="danger"
        isLoading={isRemoving}
      />
    </div>
  );
}
