// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Conversations panel for sidebar chat history display
// ABOUTME: Groups conversations by coach (session hierarchy) per Phase B Sprint C15

import { useMemo, useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { ChevronDown, ChevronRight, MessageSquare } from 'lucide-react';
import { chatApi, coachesApi } from '../../services/api';
import { QUERY_KEYS } from '../../constants/queryKeys';
import type { Conversation, Coach } from '../chat/types';
import ConversationItem from '../chat/ConversationItem';
import { ConfirmDialog } from '../ui';

interface ConversationsPanelProps {
  selectedConversation: string | null;
  onSelectConversation: (id: string | null) => void;
}

/**
 * A conversation session grouping: one bucket per coach_id, plus one
 * "no coach" bucket for unattached conversations. The session-hierarchy
 * refactor in Sprint C15 mirrors the Tier 4 backend model, where a
 * `coach_sessions` row represents exactly one (user, coach) pair and
 * owns every conversation spawned under it.
 */
interface SessionGroup {
  key: string;
  label: string;
  conversations: Conversation[];
  isNoCoach: boolean;
}

const COLLAPSED_KEY = 'dravr.conversations-panel.collapsed';

function loadCollapsedSet(): Set<string> {
  try {
    const raw = localStorage.getItem(COLLAPSED_KEY);
    if (!raw) return new Set();
    const parsed = JSON.parse(raw) as unknown;
    if (Array.isArray(parsed)) {
      return new Set(parsed.filter((v): v is string => typeof v === 'string'));
    }
  } catch {
    // fall through to empty set
  }
  return new Set();
}

function saveCollapsedSet(set: Set<string>): void {
  try {
    localStorage.setItem(COLLAPSED_KEY, JSON.stringify(Array.from(set)));
  } catch {
    // non-fatal — keep the UI working if storage is unavailable
  }
}

export default function ConversationsPanel({
  selectedConversation,
  onSelectConversation,
}: ConversationsPanelProps) {
  const [editingConversationId, setEditingConversationId] = useState<string | null>(null);
  const [editedTitleValue, setEditedTitleValue] = useState('');
  const [deleteConfirmation, setDeleteConfirmation] = useState<{ id: string; title: string } | null>(null);
  const [collapsedGroups, setCollapsedGroups] = useState<Set<string>>(() => loadCollapsedSet());
  const queryClient = useQueryClient();

  const { data: conversationsData, isLoading } = useQuery<{ conversations: Conversation[] }>({
    queryKey: QUERY_KEYS.chat.conversations(),
    queryFn: () => chatApi.getConversations(),
  });
  const conversations = useMemo(
    () => conversationsData?.conversations ?? [],
    [conversationsData?.conversations],
  );

  const { data: coachesData } = useQuery<{ coaches: Coach[] }>({
    queryKey: ['coaches', 'list'] as const,
    queryFn: () => coachesApi.list(),
    // Coach titles don't change often; cache aggressively so the sidebar
    // doesn't refetch on every conversation tab switch.
    staleTime: 5 * 60 * 1000,
  });
  const coaches = useMemo(() => coachesData?.coaches ?? [], [coachesData?.coaches]);

  const coachTitleById = useMemo(() => {
    const map = new Map<string, string>();
    for (const c of coaches) {
      map.set(c.id, c.title);
    }
    return map;
  }, [coaches]);

  const sessionGroups = useMemo<SessionGroup[]>(() => {
    const buckets = new Map<string, Conversation[]>();
    for (const conv of conversations) {
      const key = conv.coach_id ?? '__no_coach__';
      const bucket = buckets.get(key);
      if (bucket) {
        bucket.push(conv);
      } else {
        buckets.set(key, [conv]);
      }
    }

    const groups: SessionGroup[] = [];
    for (const [key, convs] of buckets.entries()) {
      if (key === '__no_coach__') continue;
      groups.push({
        key,
        label: coachTitleById.get(key) ?? 'Unknown coach',
        conversations: convs,
        isNoCoach: false,
      });
    }
    // Sort coach sessions by the most recent conversation's last_message_at desc
    groups.sort((a, b) => {
      const aLast = a.conversations[0]?.last_message_at ?? a.conversations[0]?.updated_at ?? '';
      const bLast = b.conversations[0]?.last_message_at ?? b.conversations[0]?.updated_at ?? '';
      return bLast.localeCompare(aLast);
    });

    const noCoachBucket = buckets.get('__no_coach__');
    if (noCoachBucket && noCoachBucket.length > 0) {
      groups.push({
        key: '__no_coach__',
        label: 'Without a coach',
        conversations: noCoachBucket,
        isNoCoach: true,
      });
    }

    return groups;
  }, [conversations, coachTitleById]);

  const toggleGroup = (groupKey: string): void => {
    setCollapsedGroups((prev) => {
      const next = new Set(prev);
      if (next.has(groupKey)) {
        next.delete(groupKey);
      } else {
        next.add(groupKey);
      }
      saveCollapsedSet(next);
      return next;
    });
  };

  const updateConversationMutation = useMutation({
    mutationFn: ({ id, title }: { id: string; title: string }) =>
      chatApi.updateConversation(id, { title }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() });
      setEditingConversationId(null);
      setEditedTitleValue('');
    },
  });

  const deleteConversationMutation = useMutation({
    mutationFn: (id: string) => chatApi.deleteConversation(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() });
      if (selectedConversation === deleteConfirmation?.id) {
        onSelectConversation(null);
      }
      setDeleteConfirmation(null);
    },
  });

  const handleStartRename = (e: React.MouseEvent, conv: Conversation): void => {
    e.stopPropagation();
    setEditingConversationId(conv.id);
    setEditedTitleValue(conv.title || 'Untitled Chat');
  };

  const handleSaveRename = (): void => {
    if (editingConversationId && editedTitleValue.trim()) {
      updateConversationMutation.mutate({ id: editingConversationId, title: editedTitleValue.trim() });
    } else {
      setEditingConversationId(null);
      setEditedTitleValue('');
    }
  };

  const handleCancelRename = (): void => {
    setEditingConversationId(null);
    setEditedTitleValue('');
  };

  const handleDeleteClick = (e: React.MouseEvent, conv: Conversation): void => {
    e.stopPropagation();
    setDeleteConfirmation({ id: conv.id, title: conv.title || 'Untitled Chat' });
  };

  const handleConfirmDelete = (): void => {
    if (deleteConfirmation) {
      deleteConversationMutation.mutate(deleteConfirmation.id);
    }
  };

  const handleCancelDelete = (): void => {
    setDeleteConfirmation(null);
  };

  return (
    <>
      <div className="border-t ghost-border pt-4">
        <h3 className="text-[11px] font-bold text-on-surface-variant tracking-wider uppercase px-3 mb-2">
          Coaching sessions
        </h3>
        <div className="space-y-1 max-h-96 overflow-y-auto">
          {isLoading ? (
            <div className="px-3 py-2 text-outline text-sm">Loading...</div>
          ) : sessionGroups.length === 0 ? (
            <div className="px-3 py-2 text-outline text-sm">No conversations yet</div>
          ) : (
            sessionGroups.map((group) => {
              const isCollapsed = collapsedGroups.has(group.key);
              return (
                <div
                  key={group.key}
                  className="mb-1"
                  data-testid={`session-group-${group.key}`}
                >
                  <button
                    type="button"
                    onClick={() => toggleGroup(group.key)}
                    className="group flex items-center gap-2 w-full px-3 py-1.5 rounded hover:bg-surface-container-low transition-colors"
                    aria-expanded={!isCollapsed}
                    aria-label={`Toggle ${group.label} session`}
                  >
                    {isCollapsed ? (
                      <ChevronRight className="w-3 h-3 text-outline group-hover:text-on-surface" />
                    ) : (
                      <ChevronDown className="w-3 h-3 text-outline group-hover:text-on-surface" />
                    )}
                    <MessageSquare
                      className={
                        group.isNoCoach
                          ? 'w-3 h-3 text-outline'
                          : 'w-3 h-3 text-pierre-violet'
                      }
                    />
                    <span
                      className={
                        group.isNoCoach
                          ? 'flex-1 text-left text-xs text-outline truncate'
                          : 'flex-1 text-left text-xs font-medium text-on-surface truncate'
                      }
                    >
                      {group.label}
                    </span>
                    <span className="text-[10px] text-on-surface-variant">
                      {group.conversations.length}
                    </span>
                  </button>
                  {!isCollapsed && (
                    <div className="ml-4 space-y-0.5 mt-0.5">
                      {group.conversations.slice(0, 10).map((conv) => (
                        <ConversationItem
                          key={conv.id}
                          conversation={conv}
                          isSelected={selectedConversation === conv.id}
                          isEditing={editingConversationId === conv.id}
                          editedTitleValue={editedTitleValue}
                          onSelect={() => onSelectConversation(conv.id)}
                          onStartRename={(e) => handleStartRename(e, conv)}
                          onDelete={(e) => handleDeleteClick(e, conv)}
                          onTitleChange={setEditedTitleValue}
                          onSaveRename={handleSaveRename}
                          onCancelRename={handleCancelRename}
                        />
                      ))}
                    </div>
                  )}
                </div>
              );
            })
          )}
        </div>
      </div>

      <ConfirmDialog
        isOpen={!!deleteConfirmation}
        title="Delete Conversation"
        message={`Are you sure you want to delete "${deleteConfirmation?.title}"? This action cannot be undone.`}
        confirmLabel="Delete"
        cancelLabel="Cancel"
        onConfirm={handleConfirmDelete}
        onClose={handleCancelDelete}
        variant="danger"
      />
    </>
  );
}
