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
import { defaultConversationTitle } from '@pierre/chat-utils';
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
                '-mb-px flex min-h-[44px] min-w-[44px] items-center justify-center whitespace-nowrap border-b-2 pt-1 text-sm font-medium transition-colors',
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
                'flex min-h-[44px] min-w-[44px] items-center justify-center whitespace-nowrap text-sm transition-colors',
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

      {/* Store grid */}
      <div className="p-6 sidebar-scroll">
        {isLoading ? (
          <div className="flex items-center justify-center py-12">
            <div className="text-center">
              <div className="w-8 h-8 border-2 border-primary border-t-transparent rounded-full animate-spin mx-auto" />
              <p className="mt-3 text-sm text-on-surface-variant">{t('discover.loadingCoaches')}</p>
            </div>
          </div>
        ) : isListError ? (
          <div className="text-center py-12">
            <svg
              className="w-12 h-12 text-error mx-auto mb-4"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
              aria-hidden="true"
            >
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M12 9v3.75m9-.75a9 9 0 11-18 0 9 9 0 0118 0zm-9 3.75h.008v.008H12v-.008z" />
            </svg>
            <h3 className="text-lg font-medium text-on-surface">
              {searchQuery ? t('frag.couldntSearchCoaches') : t('frag.couldntLoadStore')}
            </h3>
            <p className="text-sm text-on-surface-variant mt-1">
              {listError instanceof Error && listError.message
                ? listError.message
                : t('discover.storeListMissing')}
            </p>
            <button
              onClick={handleRetryList}
              className="btn-base btn-secondary mt-4 min-h-[44px]"
            >
              {t('discover.tryAgain')}
            </button>
          </div>
        ) : coaches.length === 0 ? (
          <div className="text-center py-16">
            <h3 className="text-lg font-medium text-on-surface">
              {searchQuery ? t('discover.noCoachesFound') : t('discover.storeEmpty')}
            </h3>
            <p className="text-sm text-on-surface-variant mt-1">
              {searchQuery
                ? t('app.noCoachesMatch', { query: searchQuery })
                : t('discover.noPublishedCoaches')}
            </p>
          </div>
        ) : (
          <>
            <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4" data-testid="store-coach-grid">
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

const CoachCard = memo(function CoachCard({ coach, onClick }: CoachCardProps) {
  const { t } = useTranslation();
  const categoryDot = COACH_CATEGORY_DOT[coach.category] ?? 'bg-outline-variant';

  return (
    <button
      onClick={onClick}
      className="text-left p-5 bg-surface-container-lowest border ghost-border rounded-xl hover:border-primary/40 transition-colors duration-200 group"
    >
      {/* Header with category and install count. The category reads the same
          vocabulary as the tabs above it, so one screen never shows the
          category in two languages. */}
      <div className="flex items-center justify-between mb-2">
        <span
          data-testid="coach-category-badge"
          className="inline-flex items-center gap-1.5 text-xs text-on-surface-variant"
        >
          <span aria-hidden="true" className={clsx('h-2 w-2 rounded-full', categoryDot)} />
          {t(coachCategoryLabelKey(coach.category))}
        </span>
        <span data-testid="coach-install-count" className="text-xs text-on-surface-variant">
          {t(coach.install_count === 1 ? 'discover.installCountOne' : 'discover.installCountN', {
            count: coach.install_count,
          })}
        </span>
      </div>

      {/* Title */}
      <h3 className="font-semibold text-on-surface mb-1 line-clamp-1 group-hover:text-primary transition-colors">
        {coach.title}
      </h3>

      {/* Description */}
      {coach.description && (
        <p className="text-sm text-on-surface-variant line-clamp-2 mb-3">{coach.description}</p>
      )}

      {/* Tags */}
      {coach.tags.length > 0 && (
        <div className="flex flex-wrap gap-1">
          {coach.tags.slice(0, 3).map((tag, index) => (
            <span
              key={index}
              className="px-2 py-0.5 text-xs bg-surface-container-low text-on-surface-variant rounded"
            >
              {tag}
            </span>
          ))}
          {coach.tags.length > 3 && (
            <span className="text-xs text-on-surface-variant">+{coach.tags.length - 3}</span>
          )}
        </div>
      )}
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
      {/* Header with back button */}
      <div className="p-4 border-b ghost-border flex items-center gap-3">
        <button
          onClick={onBack}
          title={t('discover.backToStore')}
          aria-label={t('discover.backToStore')}
          className="p-2 text-outline hover:text-on-surface hover:bg-surface-container rounded-lg transition-colors min-w-[44px] min-h-[44px] flex items-center justify-center"
        >
          <ArrowLeft className="w-5 h-5" aria-hidden="true" />
        </button>
        <h2 className="text-lg font-semibold text-on-surface truncate flex-1">{coach.title}</h2>
      </div>

      {/* Scrollable content */}
      <div className="flex-1 overflow-y-auto sidebar-scroll">
        <div className="p-6 space-y-6">
          {/* Category & Stats */}
          <div className="flex items-center justify-between">
            <span
              data-testid="coach-category-badge"
              className="inline-flex items-center gap-2 text-sm text-on-surface-variant"
            >
              <span aria-hidden="true" className={clsx('h-2 w-2 rounded-full', categoryDot)} />
              {t(coachCategoryLabelKey(coach.category))}
            </span>
            <span data-testid="coach-install-count" className="text-sm text-on-surface-variant">
              {t(coach.install_count === 1 ? 'discover.installCountOne' : 'discover.installCountN', {
                count: coach.install_count,
              })}
            </span>
          </div>

          {/* Description */}
          {coach.description && (
            <p className="text-base text-on-surface-variant leading-relaxed">{coach.description}</p>
          )}

          {/* Tags */}
          {coach.tags.length > 0 && (
            <div>
              <h3 className="text-xs font-medium text-outline mb-2">{t('discover.tagsSection')}</h3>
              <div className="flex flex-wrap gap-2">
                {coach.tags.map((tag, index) => (
                  <span
                    key={index}
                    className="px-3 py-1 text-sm bg-surface-container-low text-on-surface-variant rounded-full"
                  >
                    {tag}
                  </span>
                ))}
              </div>
            </div>
          )}

          {/* Sample Prompts */}
          {coach.sample_prompts.length > 0 && (
            <div>
              <h3 className="text-xs font-medium text-outline mb-2">{t('discover.samplePrompts')}</h3>
              <div className="space-y-2">
                {coach.sample_prompts.map((prompt, index) => (
                  <div
                    key={index}
                    className="p-3 bg-surface-container-low border ghost-border rounded-lg text-sm text-on-surface-variant"
                  >
                    {prompt}
                  </div>
                ))}
              </div>
            </div>
          )}

          {/* System Prompt Preview */}
          <div>
            <h3 className="text-xs font-medium text-outline mb-2">{t('chat.systemPromptLabel')}</h3>
            <div className="p-3 bg-surface-container-low border ghost-border rounded-lg">
              <p className="text-sm text-on-surface-variant font-mono whitespace-pre-wrap line-clamp-6">
                {coach.system_prompt}
              </p>
              {coach.system_prompt.length > 500 && (
                <p className="text-xs text-on-surface-variant italic mt-2">
                  ...and more ({coach.token_count.toLocaleString()} tokens)
                </p>
              )}
            </div>
          </div>

          {/* Details */}
          <div>
            <h3 className="text-xs font-medium text-outline mb-2">{t('discover.detailsSection')}</h3>
            <div className="bg-surface-container-low border ghost-border rounded-lg overflow-hidden">
              <div className="flex justify-between items-center px-4 py-3 border-b ghost-border">
                <span className="text-sm text-on-surface-variant">{t('discover.tokenCount')}</span>
                <span className="text-sm text-on-surface font-medium">{coach.token_count.toLocaleString()}</span>
              </div>
              {coach.published_at && (
                <div className="flex justify-between items-center px-4 py-3">
                  <span className="text-sm text-on-surface-variant">{t('discover.publishedBadge')}</span>
                  <span className="text-sm text-on-surface font-medium">
                    {new Date(coach.published_at).toLocaleDateString()}
                  </span>
                </div>
              )}
            </div>
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
      <div className="p-4 border-t ghost-border bg-surface flex gap-3">
        {isInstalled ? (
          <>
            <button
              onClick={onEdit}
              disabled={isInstalling || isOpeningChat}
              className="flex-1 py-3 px-4 bg-primary/10 text-primary font-medium rounded-lg hover:bg-primary/20 transition-colors disabled:opacity-50 disabled:cursor-not-allowed flex items-center justify-center gap-2"
            >
              <Pencil className="w-4 h-4" />
              {t('chat.editCoach')}
            </button>
            <button
              onClick={onRemove}
              disabled={isInstalling}
              className="flex-1 py-3 px-4 bg-surface-container-high border ghost-border rounded-lg text-on-surface font-medium hover:bg-surface-container-highest transition-colors disabled:opacity-50 disabled:cursor-not-allowed flex items-center justify-center gap-2"
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
            className="w-full py-3 px-4 bg-primary text-on-primary font-medium rounded-lg hover:bg-primary/80 transition-colors disabled:opacity-50 disabled:cursor-not-allowed flex items-center justify-center gap-2"
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
