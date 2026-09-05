// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
//
// ABOUTME: Discover — the Coach Store: browse, search, install, and the edit sheet for the athlete's own coaches
// ABOUTME: Installing ends with a hint that teaches /coach add @handle; editing is the one coach UI outside chat

import { useState, useEffect, useCallback, useMemo, useRef, memo } from 'react';
import { useQuery, useInfiniteQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { clsx } from 'clsx';
import { ArrowLeft, Pencil, Plus, Trash2 } from 'lucide-react';
import { chatApi, storeApi, coachesApi } from '../services/api';
import { track } from '../services/analytics';
import { SearchField, TabHeader } from './ui';
import { QUERY_KEYS } from '../constants/queryKeys';
import CoachEditSheet from './discover/CoachEditSheet';
import PostInstallHint from './discover/PostInstallHint';
import { useTranslation } from '@pierre/i18n';
import { defaultConversationTitle, initialsFor } from '@pierre/chat-utils';
import { COACH_CATEGORY_LABEL_KEY, coachCategoryLabelKey } from '@pierre/shared-constants';

// Category filter options
// Built at import time, where `t` does not exist: the table carries the key
// and the render resolves it.
const CATEGORY_FILTERS = [
  { key: 'all', labelKey: 'discover.filterAll' },
  { key: 'training', labelKey: COACH_CATEGORY_LABEL_KEY.training },
  { key: 'nutrition', labelKey: COACH_CATEGORY_LABEL_KEY.nutrition },
  { key: 'recovery', labelKey: COACH_CATEGORY_LABEL_KEY.recovery },
  { key: 'recipes', labelKey: COACH_CATEGORY_LABEL_KEY.recipes },
  { key: 'mobility', labelKey: COACH_CATEGORY_LABEL_KEY.mobility },
  { key: 'custom', labelKey: COACH_CATEGORY_LABEL_KEY.custom },
] as const;

type CategoryFilter = typeof CATEGORY_FILTERS[number]['key'];

// Sort options
const SORT_OPTIONS = [
  { key: 'popular', labelKey: 'discover.sortPopular' },
  { key: 'newest', labelKey: 'discover.sortNewest' },
  { key: 'title', labelKey: 'discover.sortAlpha' },
] as const;

type SortOption = typeof SORT_OPTIONS[number]['key'];

// A coach's pillar is an 8px dot beside its category word — meaning, not a
// coloured chip (DESIGN.md §2). Recipes sit under nutrition.
const COACH_CATEGORY_DOT: Record<string, string> = {
  training: 'bg-activity',
  nutrition: 'bg-nutrition',
  recovery: 'bg-recovery',
  recipes: 'bg-nutrition',
  mobility: 'bg-mobility',
  custom: 'bg-outline-variant',
};

interface StoreCoach {
  id: string;
  title: string;
  description: string | null;
  category: string;
  tags: string[];
  sample_prompts: string[];
  token_count: number;
  install_count: number;
  icon_url: string | null;
  published_at: string | null;
  author_id: string | null;
}

interface StoreCoachDetail extends StoreCoach {
  system_prompt: string;
  created_at: string;
  publish_status: string;
}

/** What the post-install hint teaches: the copy the install minted, by title and handle. */
interface InstalledCopy {
  title: string;
  handle: string | undefined;
}

interface StoreScreenProps {
  /**
   * Dashboard route navigator, `tab[/subview]`. t('app.openChat') on the post-install
   * hint starts a conversation and routes to `chat/<conversationId>`; closing
   * the edit sheet opened by route returns to `discover`.
   */
  onNavigate?: (route: string) => void;
  /**
   * One of the athlete's own coaches to open the edit sheet on, as the
   * `discover/<coachId>` route carries it. The sheet also opens from the
   * store detail of a listing the athlete has installed.
   */
  ownCoachId?: string | null;
}


export default function StoreScreen({ onNavigate, ownCoachId }: StoreScreenProps) {
  const { t, language } = useTranslation();
  const queryClient = useQueryClient();
  const [selectedCoachId, setSelectedCoachId] = useState<string | null>(null);
  const [selectedCategory, setSelectedCategory] = useState<CategoryFilter>('all');
  const [selectedSort, setSelectedSort] = useState<SortOption>('popular');
  const [searchQuery, setSearchQuery] = useState('');
  const [debouncedSearch, setDebouncedSearch] = useState('');
  const [successMessage, setSuccessMessage] = useState<string | null>(null);
  const [installedCopy, setInstalledCopy] = useState<InstalledCopy | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  // The edit sheet opened from a store detail; the route-driven one arrives as `ownCoachId`.
  const [detailEditCoachId, setDetailEditCoachId] = useState<string | null>(null);
  const loadMoreRef = useRef<HTMLDivElement>(null);

  // Debounce search query
  useEffect(() => {
    const timer = setTimeout(() => {
      setDebouncedSearch(searchQuery);
    }, 300);
    return () => clearTimeout(timer);
  }, [searchQuery]);

  // Clear success message after 5 seconds
  useEffect(() => {
    if (successMessage) {
      const timer = setTimeout(() => setSuccessMessage(null), 5000);
      return () => clearTimeout(timer);
    }
  }, [successMessage]);

  // Infinite query for cursor-based pagination
  const {
    data: browseData,
    isLoading: isBrowsing,
    isError: isBrowseError,
    error: browseError,
    refetch: refetchBrowse,
    fetchNextPage,
    hasNextPage,
    isFetchingNextPage,
  } = useInfiniteQuery({
    queryKey: QUERY_KEYS.store.coaches(selectedCategory, selectedSort),
    queryFn: ({ pageParam }) =>
      storeApi.browse({
        category: selectedCategory === 'all' ? undefined : selectedCategory,
        sort_by: selectedSort,
        limit: 20,
        cursor: pageParam,
      }),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (lastPage) => lastPage.has_more ? lastPage.next_cursor ?? undefined : undefined,
    enabled: !debouncedSearch,
    staleTime: 30_000,
  });

  const {
    data: searchData,
    isLoading: isSearching,
    isError: isSearchError,
    error: searchError,
    refetch: refetchSearch,
  } = useQuery({
    queryKey: QUERY_KEYS.store.search(debouncedSearch),
    queryFn: () => storeApi.search(debouncedSearch, 50),
    enabled: !!debouncedSearch,
    staleTime: 30_000,
  });

  // Fetch coach detail when selected
  const { data: coachDetail, isLoading: isLoadingDetail } = useQuery({
    queryKey: QUERY_KEYS.store.coachDetail(selectedCoachId ?? undefined),
    queryFn: () => storeApi.get(selectedCoachId!),
    enabled: !!selectedCoachId,
    staleTime: 30_000,
  });

  // Installing a store coach mints a personal copy with a fresh id and
  // `forked_from` set to the store listing's id, so the user's own coach list
  // is what maps a listing back to the copy. Same cache slot as the chat tab's
  // coach list, so an install here is visible there without a second fetch.
  const { data: myCoaches } = useQuery({
    queryKey: QUERY_KEYS.coaches.list(),
    queryFn: () => coachesApi.list(),
  });

  // Store listing id -> id of the personal copy that installing it created.
  const installedCopyBySource = useMemo(() => {
    const bySource = new Map<string, string>();
    for (const coach of myCoaches?.coaches ?? []) {
      if (coach.forked_from) {
        bySource.set(coach.forked_from, coach.id);
      }
    }
    return bySource;
  }, [myCoaches]);

  // Uninstall and edit address the personal copy, not the store listing.
  const installedCopyId = selectedCoachId
    ? installedCopyBySource.get(selectedCoachId)
    : undefined;
  const isInstalled = installedCopyId !== undefined;

  // Install mutation. The response is the minted copy, carrying the handle
  // the listing was approved with — the name the hint teaches.
  const installMutation = useMutation({
    mutationFn: (coachId: string) => storeApi.install(coachId),
    onSuccess: (installed) => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.store.installations() });
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.coaches.all });
      setActionError(null);
      setSuccessMessage(null);
      setInstalledCopy({ title: installed.coach.title, handle: installed.coach.handle });
      track({ name: 'feature_engaged', props: { feature: 'coach_installed' } });
    },
    onError: (error: Error) => {
      setInstalledCopy(null);
      setActionError(error.message || t('app.failedAddCoach'));
    },
  });

  // Uninstall mutation
  const uninstallMutation = useMutation({
    mutationFn: (coachId: string) => storeApi.uninstall(coachId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.store.installations() });
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.coaches.all });
      setActionError(null);
      setInstalledCopy(null);
      setSuccessMessage(t('app.coachRemovedFromLibrary'));
    },
    onError: (error: Error) => {
      setSuccessMessage(null);
      setActionError(error.message || t('app.failedRemoveCoach'));
    },
  });

  // t('app.openChat') on the post-install hint: a fresh conversation, then the chat
  // tab. The hint hands over the `/coach add @handle` draft the athlete types
  // there.
  const openChat = useMutation({
    // The same title the chat tab gives a fresh thread: the viewer's language,
    // the list row's 24-hour clock.
    mutationFn: () =>
      chatApi.createConversation({
        title: defaultConversationTitle(t('chat.newConversationTitlePrefix'), new Date(), language),
      }),
    onSuccess: (conversation) => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() });
      setActionError(null);
      setInstalledCopy(null);
      onNavigate?.(`chat/${encodeURIComponent(conversation.id)}`);
    },
    onError: (error: Error) => {
      setActionError(error.message || t('app.couldNotOpenChat'));
    },
  });

  const handleOpenChat = useCallback(() => {
    openChat.mutate();
  }, [openChat]);

  const handleDismissHint = useCallback(() => {
    setInstalledCopy(null);
  }, []);

  // The edit sheet: opened by the `discover/<coachId>` route or from the
  // store detail of an installed listing. Closing a route-opened sheet hands
  // the route back to Discover.
  const editingCoachId = ownCoachId ?? detailEditCoachId;
  const handleCloseEditSheet = useCallback(() => {
    setDetailEditCoachId(null);
    if (ownCoachId) {
      onNavigate?.('discover');
    }
  }, [ownCoachId, onNavigate]);

  const handleEditInstalledCopy = useCallback(() => {
    if (installedCopyId) {
      setDetailEditCoachId(installedCopyId);
    }
  }, [installedCopyId]);

  // Flatten pages for rendering
  const coaches = useMemo(() => {
    if (debouncedSearch && searchData) {
      return searchData.coaches;
    }
    return browseData?.pages.flatMap(page => page.coaches) ?? [];
  }, [debouncedSearch, searchData, browseData]);

  const isLoading = debouncedSearch ? isSearching : isBrowsing;
  const isListError = debouncedSearch ? isSearchError : isBrowseError;
  const listError = debouncedSearch ? searchError : browseError;

  const handleRetryList = useCallback(() => {
    if (debouncedSearch) {
      refetchSearch();
    } else {
      refetchBrowse();
    }
  }, [debouncedSearch, refetchSearch, refetchBrowse]);

  // Intersection Observer for infinite scroll
  useEffect(() => {
    if (debouncedSearch) return; // Don't infinite scroll for search results

    const observer = new IntersectionObserver(
      (entries) => {
        if (entries[0].isIntersecting && hasNextPage && !isFetchingNextPage) {
          fetchNextPage();
        }
      },
      { threshold: 0.1 }
    );

    if (loadMoreRef.current) {
      observer.observe(loadMoreRef.current);
    }

    return () => observer.disconnect();
  }, [hasNextPage, isFetchingNextPage, fetchNextPage, debouncedSearch]);

  const handleSelectCoach = useCallback((coachId: string) => {
    setSelectedCoachId(coachId);
  }, []);

  const handleBackToStore = useCallback(() => {
    setSelectedCoachId(null);
    setSuccessMessage(null);
    setInstalledCopy(null);
    setActionError(null);
  }, []);

  const handleInstall = useCallback(() => {
    if (selectedCoachId) {
      setActionError(null);
      installMutation.mutate(selectedCoachId);
    }
  }, [selectedCoachId, installMutation]);

  const handleRemove = useCallback(() => {
    if (installedCopyId && window.confirm(`Remove Coach?\n\nRemove "${coachDetail?.title}" from your coaches? You can always reinstall it later.`)) {
      setActionError(null);
      uninstallMutation.mutate(installedCopyId);
    }
  }, [installedCopyId, coachDetail, uninstallMutation]);

  const editSheet = editingCoachId ? (
    <CoachEditSheet coachId={editingCoachId} onClose={handleCloseEditSheet} />
  ) : null;

  // Render detail view if a coach is selected
  if (selectedCoachId) {
    return (
      <>
        <CoachDetailView
          coach={coachDetail as StoreCoachDetail | undefined}
          isLoading={isLoadingDetail}
          isInstalled={isInstalled}
          isInstalling={installMutation.isPending || uninstallMutation.isPending}
          successMessage={successMessage}
          installedCopy={installedCopy}
          isOpeningChat={openChat.isPending}
          actionError={actionError}
          onBack={handleBackToStore}
          onInstall={handleInstall}
          onRemove={handleRemove}
          onEdit={handleEditInstalledCopy}
          onOpenChat={handleOpenChat}
          onDismissHint={handleDismissHint}
        />
        {editSheet}
      </>
    );
  }

  return (
    <div className="h-full flex flex-col bg-surface">
      {editSheet}
      <TabHeader
        title={t('nav.discover')}
        description={t('discover.headerDescription')}
        actions={
          <>
            {isSearching && (
              <div
                aria-hidden="true"
                className="h-4 w-4 animate-spin rounded-full border-2 border-primary border-t-transparent"
              />
            )}
            <SearchField
              className="w-56 md:w-72"
              placeholder={t('discover.searchCoachesPlaceholder')}
              aria-label={t('discover.searchCoachesLabel')}
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
            />
          </>
        }
      />

      <div className="flex-1 overflow-y-auto min-h-0">

      {/* Categories as text tabs on the left, the sort as quiet words on the right — one row, no chips. */}
      <div className="flex items-center justify-between gap-6 border-b ghost-border px-6 overflow-x-auto">
        <div className="flex gap-5">
          {CATEGORY_FILTERS.map((filter) => (
            <button
              key={filter.key}
              onClick={() => setSelectedCategory(filter.key)}
              className={clsx(
                '-mb-px flex touch-target items-center justify-center whitespace-nowrap border-b-2 pt-1 text-sm font-medium transition-colors',
                selectedCategory === filter.key
                  ? 'border-primary text-on-surface'
                  : 'border-transparent text-on-surface-variant hover:text-on-surface'
              )}
            >
              {t(filter.labelKey)}
            </button>
          ))}
        </div>
        <div className="flex items-center gap-4">
          <span className="text-sm text-on-surface-variant whitespace-nowrap flex-shrink-0">{t('discover.sortByLabel')}</span>
          {SORT_OPTIONS.map((option) => (
            <button
              key={option.key}
              onClick={() => setSelectedSort(option.key)}
              className={clsx(
                'flex touch-target items-center justify-center whitespace-nowrap text-sm transition-colors',
                selectedSort === option.key
                  ? 'font-medium text-on-surface'
                  : 'text-on-surface-variant hover:text-on-surface'
              )}
            >
              {t(option.labelKey)}
            </button>
          ))}
        </div>
      </div>

      {/* The catalogue: a hairline list, not a grid of cards */}
      <div className="px-6 py-2 sidebar-scroll">
        {isLoading ? (
          <div className="flex items-center justify-center py-12">
            <div className="text-center">
              <div className="w-8 h-8 border-2 border-primary border-t-transparent rounded-full animate-spin mx-auto" />
              <p className="mt-3 text-sm text-on-surface-variant">{t('discover.loadingCoaches')}</p>
            </div>
          </div>
        ) : isListError ? (
          <div className="py-3">
            <h3 className="font-sans text-sm font-medium tracking-normal text-error">
              {searchQuery ? t('frag.couldntSearchCoaches') : t('frag.couldntLoadStore')}
            </h3>
            <p className="mt-0.5 text-xs text-on-surface-variant">
              {listError instanceof Error && listError.message
                ? listError.message
                : t('discover.storeListMissing')}
            </p>
            <button onClick={handleRetryList} className="btn-base btn-tertiary btn-sm mt-2 px-0">
              {t('discover.tryAgain')}
            </button>
          </div>
        ) : coaches.length === 0 ? (
          // One sentence where the rows would be, the second in the caption size.
          <div className="py-3">
            <h3 className="font-sans text-sm font-medium tracking-normal text-on-surface-variant">
              {searchQuery ? t('discover.noCoachesFound') : t('discover.storeEmpty')}
            </h3>
            <p className="mt-0.5 text-xs text-outline">
              {searchQuery
                ? t('app.noCoachesMatch', { query: searchQuery })
                : t('discover.noPublishedCoaches')}
            </p>
          </div>
        ) : (
          <>
            <div className="max-w-[1040px]" data-testid="store-coach-grid">
              {coaches.map((coach) => (
                <CoachCard key={coach.id} coach={coach} onClick={() => handleSelectCoach(coach.id)} />
              ))}
            </div>

            {/* Infinite scroll trigger */}
            {!debouncedSearch && (
              <div ref={loadMoreRef} className="py-8 flex justify-center">
                {isFetchingNextPage ? (
                  <div className="flex items-center gap-2">
                    <div className="w-5 h-5 border-2 border-primary border-t-transparent rounded-full animate-spin" />
                    <span className="text-sm text-on-surface-variant">{t('discover.loadingMore')}</span>
                  </div>
                ) : hasNextPage ? (
                  <span className="text-sm text-on-surface-variant">{t('discover.scrollForMore')}</span>
                ) : coaches.length > 0 ? (
                  <span className="text-sm text-on-surface-variant">{t('discover.endOfCoachList')}</span>
                ) : null}
              </div>
            )}
          </>
        )}
      </div>
      </div>
    </div>
  );
}

// Store coach card - memoized to prevent unnecessary re-renders during scrolling
interface CoachCardProps {
  coach: StoreCoach;
  onClick: () => void;
}

/**
 * One row of the catalogue (Boreal v2.1): a 36px initials avatar, the name
 * with its category as a dot and a word under it, one line of description,
 * and the install count in mono on the right. The v2 grid of bordered cards
 * showed the same four facts in three columns of boxes; the tags wait for
 * the detail view, where there is room to read them.
 */
const CoachCard = memo(function CoachCard({ coach, onClick }: CoachCardProps) {
  const { t } = useTranslation();
  const categoryDot = COACH_CATEGORY_DOT[coach.category] ?? 'bg-outline-variant';

  return (
    <button
      onClick={onClick}
      data-testid="coach-card"
      className="group flex min-h-[64px] w-full items-center gap-3.5 border-t ghost-border-faint py-3 text-left transition-colors first:border-t-0 hover:bg-surface-container-low/60 focus-ring"
    >
      <span
        aria-hidden="true"
        className="flex h-9 w-9 shrink-0 select-none items-center justify-center rounded-full bg-primary-container font-display text-xs font-semibold text-on-primary-container"
      >
        {initialsFor(coach.title)}
      </span>
      <span className="flex w-48 shrink-0 flex-col md:w-56">
        <h3 className="truncate font-sans text-sm font-semibold tracking-normal text-on-surface transition-colors group-hover:text-primary">
          {coach.title}
        </h3>
        {/* The category reads the same vocabulary as the tabs above it, so one
            screen never shows the category in two languages. */}
        <span
          data-testid="coach-category-badge"
          className="inline-flex items-center gap-1.5 text-xs text-on-surface-variant"
        >
          <span aria-hidden="true" className={clsx('h-1.5 w-1.5 rounded-full', categoryDot)} />
          {t(coachCategoryLabelKey(coach.category))}
        </span>
      </span>
      <span className="hidden min-w-0 flex-1 truncate text-sm text-on-surface-variant md:inline">
        {coach.description ?? ''}
      </span>
      {/* Tags are words in the caption size, not chips; the first three, on a wide screen. */}
      {coach.tags.length > 0 && (
        <span className="hidden shrink-0 gap-1.5 text-xs text-outline lg:inline-flex">
          {coach.tags.slice(0, 3).map((tag, index) => (
            <span key={tag}>
              {index > 0 ? <span aria-hidden="true">· </span> : null}
              <span>{tag}</span>
            </span>
          ))}
        </span>
      )}
      <span data-testid="coach-install-count" className="ml-auto shrink-0 font-mono text-xs text-outline">
        {t(coach.install_count === 1 ? 'discover.installCountOne' : 'discover.installCountN', {
          count: coach.install_count,
        })}
      </span>
    </button>
  );
});

// Coach detail view component
interface CoachDetailViewProps {
  coach: StoreCoachDetail | undefined;
  isLoading: boolean;
  isInstalled: boolean;
  isInstalling: boolean;
  successMessage: string | null;
  /** Set right after an install; the hint that teaches the coach's handle. */
  installedCopy: InstalledCopy | null;
  isOpeningChat: boolean;
  actionError: string | null;
  onBack: () => void;
  onInstall: () => void;
  onRemove: () => void;
  /** Open the edit sheet on the athlete's installed copy. */
  onEdit: () => void;
  onOpenChat: (draft: string) => void;
  onDismissHint: () => void;
}

function CoachDetailView({
  coach,
  isLoading,
  isInstalled,
  isInstalling,
  successMessage,
  installedCopy,
  isOpeningChat,
  actionError,
  onBack,
  onInstall,
  onRemove,
  onEdit,
  onOpenChat,
  onDismissHint,
}: CoachDetailViewProps) {
  const { t } = useTranslation();
  if (isLoading) {
    return (
      <div className="h-full flex flex-col bg-surface">
        <div className="flex-1 flex items-center justify-center">
          <div className="text-center">
            <div className="w-8 h-8 border-2 border-primary border-t-transparent rounded-full animate-spin mx-auto" />
            <p className="mt-3 text-sm text-on-surface-variant">{t('discover.loadingCoachDetails')}</p>
          </div>
        </div>
      </div>
    );
  }

  if (!coach) {
    return (
      <div className="h-full flex flex-col bg-surface">
        <div className="flex-1 flex items-center justify-center">
          <div className="text-center">
            <p className="text-lg text-on-surface-variant mb-4">{t('discover.coachNotFound')}</p>
            <button
              onClick={onBack}
              className="btn-base btn-primary"
            >
              {t('discover.goBack')}
            </button>
          </div>
        </div>
      </div>
    );
  }

  const categoryDot = COACH_CATEGORY_DOT[coach.category] ?? 'bg-outline-variant';

  return (
    <div className="h-full flex flex-col bg-surface">
      {/* Header with back button — the same 52px row as every page */}
      <div className="flex h-[52px] items-center gap-2 border-b ghost-border px-4">
        <button
          onClick={onBack}
          title={t('discover.backToStore')}
          aria-label={t('discover.backToStore')}
          className="flex h-8 w-8 items-center justify-center rounded-lg text-outline transition-colors hover:bg-surface-container-low hover:text-on-surface touch-target"
        >
          <ArrowLeft className="h-4 w-4" aria-hidden="true" />
        </button>
        <h2 className="min-w-0 flex-1 truncate font-display text-xl font-semibold text-on-surface">{coach.title}</h2>
      </div>

      {/* Scrollable content — one reading column, sections set apart by space, no boxes */}
      <div className="flex-1 overflow-y-auto sidebar-scroll">
        <div className="max-w-[720px] space-y-8 px-6 py-5">
          {/* Category & Stats */}
          <div className="flex items-center justify-between">
            <span
              data-testid="coach-category-badge"
              className="inline-flex items-center gap-2 text-sm text-on-surface-variant"
            >
              <span aria-hidden="true" className={clsx('h-1.5 w-1.5 rounded-full', categoryDot)} />
              {t(coachCategoryLabelKey(coach.category))}
            </span>
            <span data-testid="coach-install-count" className="font-mono text-xs text-outline">
              {t(coach.install_count === 1 ? 'discover.installCountOne' : 'discover.installCountN', {
                count: coach.install_count,
              })}
            </span>
          </div>

          {/* Description */}
          {coach.description && (
            <p className="text-base leading-relaxed text-on-surface">{coach.description}</p>
          )}

          {/* Tags — words separated by dots, not chips */}
          {coach.tags.length > 0 && (
            <div>
              <h3 className="font-sans text-sm font-semibold tracking-normal text-on-surface">{t('discover.tagsSection')}</h3>
              <p className="mt-1.5 text-sm text-on-surface-variant">
                {coach.tags.map((tag, index) => (
                  <span key={index}>
                    {index > 0 ? <span aria-hidden="true" className="text-outline"> · </span> : null}
                    <span>{tag}</span>
                  </span>
                ))}
              </p>
            </div>
          )}

          {/* Sample Prompts — a hairline list */}
          {coach.sample_prompts.length > 0 && (
            <div>
              <h3 className="font-sans text-sm font-semibold tracking-normal text-on-surface">{t('discover.samplePrompts')}</h3>
              <ul className="mt-1.5">
                {coach.sample_prompts.map((prompt, index) => (
                  <li key={index} className="border-t ghost-border-faint py-2.5 text-sm text-on-surface-variant first:border-t-0">
                    {prompt}
                  </li>
                ))}
              </ul>
            </div>
          )}

          {/* System Prompt Preview — the one framed object on the page: it is code */}
          <div>
            <h3 className="font-sans text-sm font-semibold tracking-normal text-on-surface">{t('chat.systemPromptLabel')}</h3>
            <div className="mt-1.5 rounded-lg border ghost-border-faint bg-surface-container-lowest p-3">
              <p className="font-mono text-xs leading-relaxed text-on-surface-variant whitespace-pre-wrap line-clamp-6">
                {coach.system_prompt}
              </p>
              {coach.system_prompt.length > 500 && (
                <p className="mt-2 text-xs italic text-outline">
                  ...and more ({coach.token_count.toLocaleString()} tokens)
                </p>
              )}
            </div>
          </div>

          {/* Details — label and value on one line, a faint rule between */}
          <div>
            <h3 className="font-sans text-sm font-semibold tracking-normal text-on-surface">{t('discover.detailsSection')}</h3>
            <dl className="mt-1.5">
              <div className="flex items-center justify-between py-2">
                <dt className="text-sm text-on-surface-variant">{t('discover.tokenCount')}</dt>
                <dd className="font-mono text-sm text-on-surface">{coach.token_count.toLocaleString()}</dd>
              </div>
              {coach.published_at && (
                <div className="flex items-center justify-between border-t ghost-border-faint py-2">
                  <dt className="text-sm text-on-surface-variant">{t('discover.publishedBadge')}</dt>
                  <dd className="font-mono text-sm text-on-surface">{new Date(coach.published_at).toLocaleDateString()}</dd>
                </div>
              )}
            </dl>
          </div>

          {/* Install/remove failure */}
          {actionError && (
            <div className="p-4 bg-error/10 border border-error/30 rounded-lg">
              <p className="text-sm text-error">{actionError}</p>
            </div>
          )}

          {/* After an install: the hint that teaches how to use the coach from
              any chat. Discover keeps no coach list of its own. */}
          {installedCopy && (
            <PostInstallHint
              coachTitle={installedCopy.title}
              handle={installedCopy.handle}
              onOpenChat={onOpenChat}
              onDismiss={onDismissHint}
            />
          )}

          {/* Removal confirmation */}
          {successMessage && (
            <div className="p-4 bg-success/20 border border-success/30 rounded-lg">
              <p className="text-sm text-success">{successMessage}</p>
            </div>
          )}

          {/* Bottom spacer for fixed button */}
          <div className="h-20" />
        </div>
      </div>

      {/* Fixed action bar at bottom. An installed listing is the athlete's own
          copy, so it can be edited from here — the one coach editor outside chat. */}
      <div className="flex items-center justify-end gap-2 border-t ghost-border bg-surface px-6 py-3">
        {isInstalled ? (
          <>
            <button
              onClick={onEdit}
              disabled={isInstalling || isOpeningChat}
              className="btn-base btn-tertiary gap-1.5 disabled:cursor-not-allowed disabled:opacity-50"
            >
              <Pencil className="h-4 w-4" />
              {t('chat.editCoach')}
            </button>
            <button
              onClick={onRemove}
              disabled={isInstalling}
              className="btn-base btn-secondary gap-1.5 disabled:cursor-not-allowed disabled:opacity-50"
            >
              {isInstalling ? (
                <div className="w-5 h-5 border-2 border-white border-t-transparent rounded-full animate-spin" />
              ) : (
                <>
                  <Trash2 className="w-4 h-4" />
                  {t('discover.removeCoach')}
                </>
              )}
            </button>
          </>
        ) : (
          <button
            onClick={onInstall}
            disabled={isInstalling}
            className="btn-base btn-primary gap-1.5 disabled:cursor-not-allowed disabled:opacity-50"
          >
            {isInstalling ? (
              <div className="w-5 h-5 border-2 border-white border-t-transparent rounded-full animate-spin" />
            ) : (
              <>
                <Plus className="w-4 h-4" />
                {t('discover.addCoach')}
              </>
            )}
          </button>
        )}
      </div>
    </div>
  );
}
