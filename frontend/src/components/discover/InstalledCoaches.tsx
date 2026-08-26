// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: "Your coaches" — the athlete's installed coaches pinned at the top of Discover, each with its @handle
// ABOUTME: Its own query over the coach list, never a re-rank of the store page; carries the coach-library actions

import { useMemo, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { clsx } from 'clsx';
import { Eye, EyeOff, Plus } from 'lucide-react';
import type { Coach } from '@pierre/shared-types';
import { chatApi, coachesApi, storeApi } from '../../services/api';
import { QUERY_KEYS } from '../../constants/queryKeys';
import { Button, Card, ConfirmDialog } from '../ui';
import {
  CoachFormModal,
  DEFAULT_COACH_FORM_DATA,
  coachToFormData,
  formDataToCreateRequest,
  formDataToUpdateRequest,
} from '../chat';
import type { CoachFormData } from '../chat';
import CoachImport from './CoachImport';
import InstalledCoachDetail from './InstalledCoachDetail';
import { categoryAccent, categoryBadgeClass, categoryEmoji } from './coachCategory';

export interface InstalledCoachesProps {
  /**
   * The Discover search text. One box searches both lists, so a query that
   * narrows the store also narrows the coaches the athlete already has.
   */
  searchQuery: string;
  /**
   * Dashboard route navigator, `tab[/subview]`. "Chat" opens a conversation
   * with the coach and routes to `chat/<conversationId>`.
   */
  onNavigate?: (route: string) => void;
}

/** Card action buttons are small but keep the 44px touch target. */
const ACTION_CLASS = 'min-h-[44px]';

/** The default title a fresh conversation gets, matching the chat tab's own. */
function defaultConversationTitle(): string {
  const now = new Date();
  const day = now.toLocaleDateString('en-US', { month: 'short', day: 'numeric' });
  const time = now.toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' });
  return `Chat ${day} ${time}`;
}

/**
 * The athlete's coach list, pinned above the store.
 *
 * "Installed" is what the server means by it — every coach on the athlete's
 * list: assigned system coaches, personal coaches, and copies installed from
 * the store. That is the set a `@handle` resolves against, so each card shows
 * the handle the athlete types to bring that coach into a conversation.
 *
 * This is a separate query, deliberately: the store page is re-ranked per
 * cursor page, so a coach on its third page could never surface at the top
 * of the first. The list here does not come from the store at all.
 */
export default function InstalledCoaches({ searchQuery, onNavigate }: InstalledCoachesProps) {
  const queryClient = useQueryClient();
  const [showHidden, setShowHidden] = useState(false);
  const [selectedCoach, setSelectedCoach] = useState<Coach | null>(null);
  const [editorOpen, setEditorOpen] = useState(false);
  const [editingCoachId, setEditingCoachId] = useState<string | null>(null);
  const [formData, setFormData] = useState<CoachFormData>(DEFAULT_COACH_FORM_DATA);
  const [removeTarget, setRemoveTarget] = useState<Coach | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);

  const {
    data: coachesData,
    isLoading,
    isError,
    error,
    refetch,
  } = useQuery({
    queryKey: QUERY_KEYS.coaches.listWithHidden(),
    queryFn: () => coachesApi.list({ include_hidden: true, personalize: true }),
  });

  const { data: hiddenData } = useQuery({
    queryKey: QUERY_KEYS.coaches.hidden(),
    queryFn: () => coachesApi.getHidden(),
  });

  const invalidateCoaches = () => {
    queryClient.invalidateQueries({ queryKey: QUERY_KEYS.coaches.all });
    queryClient.invalidateQueries({ queryKey: QUERY_KEYS.store.installations() });
  };

  const failWith = (fallback: string) => (err: Error) => setActionError(err.message || fallback);

  const createCoach = useMutation({
    mutationFn: (data: CoachFormData) => coachesApi.create(formDataToCreateRequest(data)),
    onSuccess: () => {
      invalidateCoaches();
      setActionError(null);
      closeEditor();
    },
    onError: failWith('Failed to create coach'),
  });

  const updateCoach = useMutation({
    mutationFn: ({ id, data }: { id: string; data: CoachFormData }) =>
      coachesApi.update(id, formDataToUpdateRequest(data)),
    onSuccess: (updated) => {
      invalidateCoaches();
      setActionError(null);
      closeEditor();
      setSelectedCoach((prev) => (prev && prev.id === updated.id ? { ...prev, ...updated } : prev));
    },
    onError: failWith('Failed to save coach'),
  });

  // A copy installed from the store is uninstalled through the store so its
  // listing's install count stays honest; a personal coach is simply deleted.
  const removeCoach = useMutation({
    mutationFn: async (coach: Coach) => {
      if (coach.forked_from) {
        await storeApi.uninstall(coach.id);
      } else {
        await coachesApi.delete(coach.id);
      }
    },
    onSuccess: () => {
      invalidateCoaches();
      setActionError(null);
      setRemoveTarget(null);
      setSelectedCoach(null);
    },
    onError: (err: Error) => {
      setRemoveTarget(null);
      setActionError(err.message || 'Failed to remove coach');
    },
  });

  const toggleFavorite = useMutation({
    mutationFn: (id: string) => coachesApi.toggleFavorite(id),
    onSuccess: (data, id) => {
      invalidateCoaches();
      setActionError(null);
      setSelectedCoach((prev) => (prev && prev.id === id ? { ...prev, is_favorite: data.is_favorite } : prev));
    },
    onError: failWith('Failed to update favorite'),
  });

  const hideCoach = useMutation({
    mutationFn: (id: string) => coachesApi.hide(id),
    onSuccess: () => {
      invalidateCoaches();
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.coaches.hidden() });
      setActionError(null);
    },
    onError: failWith('Failed to hide coach'),
  });

  const showCoach = useMutation({
    mutationFn: (id: string) => coachesApi.show(id),
    onSuccess: () => {
      invalidateCoaches();
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.coaches.hidden() });
      setActionError(null);
    },
    onError: failWith('Failed to show coach'),
  });

  const forkCoach = useMutation({
    mutationFn: (id: string) => coachesApi.fork(id),
    onSuccess: () => {
      invalidateCoaches();
      setActionError(null);
    },
    onError: failWith('Failed to fork coach'),
  });

  const exportCoach = useMutation({
    mutationFn: (coachId: string) => coachesApi.exportAsMarkdown(coachId),
    onSuccess: (markdown, coachId) => {
      const coach = (coachesData?.coaches ?? []).find((c) => c.id === coachId);
      const filename = `${(coach?.title || 'coach').toLowerCase().replace(/\s+/g, '-')}.md`;
      const blob = new Blob([markdown], { type: 'text/markdown' });
      const url = URL.createObjectURL(blob);
      const link = document.createElement('a');
      link.href = url;
      link.download = filename;
      document.body.appendChild(link);
      link.click();
      document.body.removeChild(link);
      URL.revokeObjectURL(url);
      setActionError(null);
    },
    onError: failWith('Failed to export coach'),
  });

  // Chat with this coach: open a conversation bound to it and route there.
  const startChat = useMutation({
    mutationFn: (coach: Coach) =>
      chatApi.createConversation({ title: defaultConversationTitle(), coach_id: coach.id }),
    onSuccess: (conversation) => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() });
      setActionError(null);
      setSelectedCoach(null);
      onNavigate?.(`chat/${encodeURIComponent(conversation.id)}`);
    },
    onError: failWith('Could not start a chat with this coach'),
  });

  const hiddenIds = useMemo(
    () => new Set((hiddenData?.coaches ?? []).map((c) => c.id)),
    [hiddenData],
  );

  const coaches = useMemo(() => {
    const query = searchQuery.trim().toLowerCase();
    return (coachesData?.coaches ?? [])
      .map((coach) => ({ ...coach, is_hidden: hiddenIds.has(coach.id) }))
      .filter((coach) => showHidden || !coach.is_hidden)
      .filter((coach) => {
        if (!query) return true;
        return (
          coach.title.toLowerCase().includes(query) ||
          (coach.description ?? '').toLowerCase().includes(query) ||
          (coach.handle ?? '').includes(query)
        );
      })
      .sort((a, b) => {
        if (a.is_favorite !== b.is_favorite) return a.is_favorite ? -1 : 1;
        return b.use_count - a.use_count;
      });
  }, [coachesData, hiddenIds, showHidden, searchQuery]);

  const totalCount = coachesData?.coaches?.length ?? 0;

  const openCreate = () => {
    setEditingCoachId(null);
    setFormData(DEFAULT_COACH_FORM_DATA);
    setEditorOpen(true);
  };

  // The editor is a modal of its own; a detail sheet left open under it
  // would swallow the clicks meant for the editor.
  const openEdit = (coach: Coach) => {
    setSelectedCoach(null);
    setEditingCoachId(coach.id);
    setFormData(coachToFormData(coach));
    setEditorOpen(true);
  };

  const closeEditor = () => {
    setEditorOpen(false);
    setEditingCoachId(null);
    setFormData(DEFAULT_COACH_FORM_DATA);
  };

  const submitEditor = () => {
    if (editingCoachId) updateCoach.mutate({ id: editingCoachId, data: formData });
    else createCoach.mutate(formData);
  };

  const renderCard = (coach: Coach & { is_hidden: boolean }) => (
    <div
      key={coach.id}
      data-testid="installed-coach-card"
      className={clsx(
        'bg-surface-container-low rounded-xl p-4 border ghost-border border-l-4 transition-all hover:shadow-md',
        coach.is_hidden && 'opacity-60',
      )}
      style={{ borderLeftColor: categoryAccent(coach.category) }}
    >
      <div className="flex items-start gap-3">
        <button
          type="button"
          onClick={() => setSelectedCoach(coach)}
          className="w-12 h-12 rounded-xl flex items-center justify-center flex-shrink-0 text-xl"
          style={{ backgroundColor: `${categoryAccent(coach.category)}20` }}
          aria-label={`Open ${coach.title}`}
        >
          <span aria-hidden="true">{categoryEmoji(coach.category)}</span>
        </button>
        <div className="flex-1 min-w-0">
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1 mb-1">
            <h3 className={clsx('font-semibold min-w-0 break-words', coach.is_hidden ? 'text-on-surface-variant' : 'text-on-surface')}>
              <button
                type="button"
                onClick={() => setSelectedCoach(coach)}
                className="text-left hover:text-primary transition-colors min-h-[44px] -my-2 py-2"
              >
                {coach.title}
              </button>
            </h3>
            <span
              className={clsx(
                'px-2 py-0.5 text-xs font-medium rounded-full border flex-shrink-0 capitalize',
                categoryBadgeClass(coach.category),
              )}
            >
              {coach.category}
            </span>
            {coach.is_system && (
              <span className="px-2 py-0.5 text-xs font-medium rounded-full bg-primary/10 text-primary border border-primary/20 flex-shrink-0">
                System
              </span>
            )}
            {coach.is_hidden && (
              <span className="px-2 py-0.5 text-xs font-medium rounded-full bg-surface-container-high text-on-surface-variant flex-shrink-0">
                Hidden
              </span>
            )}
          </div>
          {/* The handle is the name that brings this coach into any
              conversation: `@handle` in a message routes that turn to it. */}
          {coach.handle && (
            <p className="font-mono text-xs text-primary mb-1" data-testid="coach-handle">
              @{coach.handle}
            </p>
          )}
          {coach.description && (
            <p className="text-sm text-on-surface-variant line-clamp-3">{coach.description}</p>
          )}
        </div>
        <button
          type="button"
          onClick={() => toggleFavorite.mutate(coach.id)}
          className="min-w-[44px] min-h-[44px] flex items-center justify-center text-outline hover:text-warning transition-colors flex-shrink-0"
          title={coach.is_favorite ? 'Remove from favorites' : 'Add to favorites'}
          aria-label={coach.is_favorite ? 'Remove from favorites' : 'Add to favorites'}
        >
          <svg
            className={clsx('w-4 h-4', coach.is_favorite ? 'fill-warning text-warning' : 'fill-none')}
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

      <div className="flex flex-wrap items-center justify-end mt-3 pt-2 border-t ghost-border gap-1">
        <Button size="sm" className={ACTION_CLASS} onClick={() => startChat.mutate(coach)} disabled={startChat.isPending}>
          Chat
        </Button>
        {!coach.is_system && (
          <Button size="sm" variant="secondary" className={ACTION_CLASS} onClick={() => openEdit(coach)}>
            Edit
          </Button>
        )}
        <Button
          size="sm"
          variant="secondary"
          className={ACTION_CLASS}
          onClick={() => exportCoach.mutate(coach.id)}
          disabled={exportCoach.isPending}
        >
          {exportCoach.isPending ? 'Exporting...' : 'Export'}
        </Button>
        {coach.is_system && (
          <Button
            size="sm"
            variant="secondary"
            className={ACTION_CLASS}
            onClick={() => forkCoach.mutate(coach.id)}
            disabled={forkCoach.isPending}
          >
            Fork
          </Button>
        )}
        {coach.is_system && (
          <Button
            size="sm"
            variant="secondary"
            className={ACTION_CLASS}
            onClick={() => (coach.is_hidden ? showCoach.mutate(coach.id) : hideCoach.mutate(coach.id))}
            disabled={hideCoach.isPending || showCoach.isPending}
          >
            {coach.is_hidden ? 'Show' : 'Hide'}
          </Button>
        )}
        {!coach.is_system && (
          <Button size="sm" variant="danger" className={ACTION_CLASS} onClick={() => setRemoveTarget(coach)}>
            {coach.forked_from ? 'Remove' : 'Delete'}
          </Button>
        )}
      </div>
    </div>
  );

  return (
    <section aria-labelledby="your-coaches-heading" className="px-6 py-4 border-b ghost-border">
      <div className="flex items-center justify-between gap-3 mb-3">
        <div className="min-w-0">
          <h2 id="your-coaches-heading" className="text-sm font-semibold text-on-surface-variant uppercase tracking-wide">
            Your coaches{isLoading ? '' : ` (${totalCount})`}
          </h2>
          <p className="text-xs text-on-surface-variant">Type a coach&apos;s @handle in any chat to bring it into the conversation.</p>
        </div>
        <div className="flex items-center gap-1 flex-shrink-0">
          <button
            type="button"
            onClick={() => setShowHidden((v) => !v)}
            aria-pressed={showHidden}
            className={clsx(
              'p-2 rounded-lg transition-colors min-w-[44px] min-h-[44px] flex items-center justify-center',
              showHidden ? 'bg-primary/20 text-primary' : 'text-outline hover:text-on-surface hover:bg-surface-container-low',
            )}
            title={showHidden ? 'Hide hidden coaches' : 'Show hidden coaches'}
            aria-label={showHidden ? 'Hide hidden coaches' : 'Show hidden coaches'}
          >
            {showHidden ? <Eye className="w-5 h-5" aria-hidden="true" /> : <EyeOff className="w-5 h-5" aria-hidden="true" />}
          </button>
          <CoachImport />
          <button
            type="button"
            onClick={openCreate}
            className="p-2 rounded-lg text-on-primary bg-primary hover:bg-primary-container transition-colors shadow-ambient min-w-[44px] min-h-[44px] flex items-center justify-center"
            title="Create Coach"
            aria-label="Create Coach"
          >
            <Plus className="w-4 h-4" aria-hidden="true" />
          </button>
        </div>
      </div>

      {actionError && (
        <div className="mb-3 p-3 rounded-lg bg-error/10 border border-error/30">
          <p className="text-sm text-error">{actionError}</p>
        </div>
      )}

      {isLoading ? (
        <div className="flex justify-center py-6">
          <div className="pierre-spinner w-6 h-6" />
        </div>
      ) : isError ? (
        <Card variant="dark" className="text-center py-8">
          <h3 className="text-lg font-medium text-on-surface mb-2">Couldn&apos;t load your coaches</h3>
          <p className="text-on-surface-variant mb-4">
            {error instanceof Error && error.message ? error.message : 'The server did not return your coach list.'}
          </p>
          <Button onClick={() => refetch()}>Try Again</Button>
        </Card>
      ) : coaches.length === 0 ? (
        <Card variant="dark" className="text-center py-8">
          <h3 className="text-lg font-medium text-on-surface mb-2">
            {totalCount === 0 ? 'No coaches yet' : 'No coaches match'}
          </h3>
          <p className="text-on-surface-variant mb-4">
            {totalCount === 0
              ? 'Add a coach from the store below, or create your own.'
              : 'Try a different search, or show hidden coaches.'}
          </p>
          {totalCount === 0 && <Button onClick={openCreate}>Create Your First Coach</Button>}
        </Card>
      ) : (
        <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">{coaches.map(renderCard)}</div>
      )}

      <InstalledCoachDetail
        coach={selectedCoach}
        onClose={() => setSelectedCoach(null)}
        onChat={(coach) => startChat.mutate(coach)}
        onEdit={openEdit}
        onRemove={setRemoveTarget}
        onToggleFavorite={(coach) => toggleFavorite.mutate(coach.id)}
        actionError={actionError}
        isRemoving={removeCoach.isPending}
      />

      <CoachFormModal
        isOpen={editorOpen}
        isEditing={editingCoachId !== null}
        formData={formData}
        onFormDataChange={setFormData}
        onSubmit={submitEditor}
        onClose={closeEditor}
        isSubmitting={editingCoachId ? updateCoach.isPending : createCoach.isPending}
        submitError={createCoach.isError || updateCoach.isError}
      />

      <ConfirmDialog
        isOpen={removeTarget !== null}
        onClose={() => setRemoveTarget(null)}
        onConfirm={() => {
          if (removeTarget) removeCoach.mutate(removeTarget);
        }}
        title={removeTarget?.forked_from ? 'Remove Coach?' : 'Delete Coach?'}
        message={
          removeTarget?.forked_from
            ? `Remove "${removeTarget.title}" from your coaches? You can always add it again from the store.`
            : `Delete coach "${removeTarget?.title ?? ''}"? This cannot be undone.`
        }
        confirmLabel={removeTarget?.forked_from ? 'Remove' : 'Delete'}
        cancelLabel="Cancel"
        variant="danger"
        isLoading={removeCoach.isPending}
      />
    </section>
  );
}
