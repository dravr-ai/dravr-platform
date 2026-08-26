// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
//
// ABOUTME: Discover — the athlete's own coaches pinned above the Coach Store browse grid
// ABOUTME: Lists published coaches with category filters, search, and detail view with install/uninstall

import { useState, useEffect, useCallback, useMemo, useRef, memo } from 'react';
import { useQuery, useInfiniteQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { clsx } from 'clsx';
import { Compass, ArrowLeft, Plus, Trash2 } from 'lucide-react';
import { storeApi, coachesApi } from '../services/api';
import { track } from '../services/analytics';
import { TabHeader } from './ui';
import { QUERY_KEYS } from '../constants/queryKeys';
import InstalledCoaches from './discover/InstalledCoaches';

// Category filter options
const CATEGORY_FILTERS = [
  { key: 'all', label: 'All' },
  { key: 'training', label: 'Training' },
  { key: 'nutrition', label: 'Nutrition' },
  { key: 'recovery', label: 'Recovery' },
  { key: 'recipes', label: 'Recipes' },
  { key: 'mobility', label: 'Mobility' },
  { key: 'custom', label: 'Custom' },
] as const;

type CategoryFilter = typeof CATEGORY_FILTERS[number]['key'];

// Sort options
const SORT_OPTIONS = [
  { key: 'popular', label: 'Popular' },
  { key: 'newest', label: 'Newest' },
  { key: 'title', label: 'A-Z' },
] as const;

type SortOption = typeof SORT_OPTIONS[number]['key'];

// Coach category colors (dark theme)
const COACH_CATEGORY_COLORS: Record<string, string> = {
  training: 'bg-activity/20 text-activity',
  nutrition: 'bg-nutrition/20 text-nutrition',
  recovery: 'bg-recovery/20 text-recovery',
  recipes: 'bg-nutrition/20 text-nutrition',
  mobility: 'bg-mobility/20 text-mobility',
  custom: 'bg-surface-container-high text-on-surface-variant',
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

interface StoreScreenProps {
  /**
   * Dashboard route navigator, `tab[/subview]`. Chatting with one of the
   * pinned coaches opens a conversation and routes to `chat/<conversationId>`.
   */
  onNavigate?: (route: string) => void;
}

export default function StoreScreen({ onNavigate }: StoreScreenProps) {
  const queryClient = useQueryClient();
  const [selectedCoachId, setSelectedCoachId] = useState<string | null>(null);
  const [selectedCategory, setSelectedCategory] = useState<CategoryFilter>('all');
  const [selectedSort, setSelectedSort] = useState<SortOption>('popular');
  const [searchQuery, setSearchQuery] = useState('');
  const [debouncedSearch, setDebouncedSearch] = useState('');
  const [successMessage, setSuccessMessage] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
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
  // is what maps a listing back to the copy. Query options mirror
  // InstalledCoaches so both components share the `listWithHidden` cache slot.
  const { data: myCoaches } = useQuery({
    queryKey: QUERY_KEYS.coaches.listWithHidden(),
    queryFn: () => coachesApi.list({
      include_hidden: true,
      personalize: true,
    }),
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

  // Uninstall addresses the personal copy, not the store listing.
  const installedCopyId = selectedCoachId
    ? installedCopyBySource.get(selectedCoachId)
    : undefined;
  const isInstalled = installedCopyId !== undefined;

  // Install mutation
  const installMutation = useMutation({
    mutationFn: (coachId: string) => storeApi.install(coachId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.store.installations() });
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.coaches.all });
      setActionError(null);
      setSuccessMessage(`"${coachDetail?.title}" has been added to your coaches.`);
      track({ name: 'feature_engaged', props: { feature: 'coach_installed' } });
    },
    onError: (error: Error) => {
      setSuccessMessage(null);
      setActionError(error.message || 'Failed to add coach');
    },
  });

  // Uninstall mutation
  const uninstallMutation = useMutation({
    mutationFn: (coachId: string) => storeApi.uninstall(coachId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.store.installations() });
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.coaches.all });
      setActionError(null);
      setSuccessMessage(`Coach has been removed from your library.`);
    },
    onError: (error: Error) => {
      setSuccessMessage(null);
      setActionError(error.message || 'Failed to remove coach');
    },
  });

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

  const handleClearSearch = useCallback(() => {
    setSearchQuery('');
    setDebouncedSearch('');
  }, []);

  const handleSelectCoach = useCallback((coachId: string) => {
    setSelectedCoachId(coachId);
  }, []);

  const handleBackToStore = useCallback(() => {
    setSelectedCoachId(null);
    setSuccessMessage(null);
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

  // Render detail view if a coach is selected
  if (selectedCoachId) {
    return (
      <CoachDetailView
        coach={coachDetail as StoreCoachDetail | undefined}
        isLoading={isLoadingDetail}
        isInstalled={isInstalled}
        isInstalling={installMutation.isPending || uninstallMutation.isPending}
        successMessage={successMessage}
        actionError={actionError}
        onBack={handleBackToStore}
        onInstall={handleInstall}
        onRemove={handleRemove}
      />
    );
  }

  return (
    <div className="h-full flex flex-col bg-surface">
      <TabHeader
        icon={<Compass className="w-5 h-5" />}
        gradient="from-activity to-activity"
        description="Find AI coaching assistants"
      />

      <div className="flex-1 overflow-y-auto min-h-0">

      {/* Search Bar */}
      <div className="px-6 py-4 border-b ghost-border">
        <div className="relative">
          <svg
            className="absolute left-3 top-1/2 transform -translate-y-1/2 w-5 h-5 text-on-surface-variant"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24"
            aria-hidden="true"
          >
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
          </svg>
          <input
            type="search"
            placeholder="Search coaches..."
            aria-label="Search coaches"
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            className="w-full pl-10 pr-10 py-2.5 bg-surface-container-low border ghost-border rounded-lg text-sm text-on-surface placeholder-outline focus:outline-none focus:ring-2 focus:ring-primary/30 focus:border-primary transition-colors"
          />
          {searchQuery && (
            <button
              onClick={handleClearSearch}
              aria-label="Clear search"
              className="absolute right-1 top-1/2 transform -translate-y-1/2 text-on-surface-variant hover:text-outline min-w-[44px] min-h-[44px] flex items-center justify-center"
            >
              <svg className="w-5 h-5" aria-hidden="true" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
              </svg>
            </button>
          )}
          {isSearching && (
            <div className="absolute right-3 top-1/2 transform -translate-y-1/2" aria-hidden="true">
              <div className="w-4 h-4 border-2 border-primary border-t-transparent rounded-full animate-spin" />
            </div>
          )}
        </div>
      </div>

      {/* The athlete's own coaches, pinned above the store. Its own query —
          never a re-rank of the browse page, which is ranked per cursor page. */}
      <InstalledCoaches searchQuery={debouncedSearch} onNavigate={onNavigate} />

      {/* Category Filters */}
      <div className="px-6 py-3 border-b ghost-border overflow-x-auto">
        <div className="flex items-center gap-2">
          {CATEGORY_FILTERS.map((filter) => (
            <button
              key={filter.key}
              onClick={() => setSelectedCategory(filter.key)}
              className={clsx(
                'px-4 py-1.5 text-sm font-medium rounded-full whitespace-nowrap transition-colors min-h-[44px] flex items-center',
                selectedCategory === filter.key
                  ? 'bg-primary text-on-primary shadow-ambient'
                  : 'bg-surface-container-low text-on-surface-variant hover:bg-surface-container hover:text-on-surface'
              )}
            >
              {filter.label}
            </button>
          ))}
        </div>
      </div>

      {/* Sort Options */}
      <div className="px-6 py-2 bg-surface-container-low border-b ghost-border flex items-center gap-3 overflow-x-auto">
        <span className="text-sm text-on-surface-variant whitespace-nowrap flex-shrink-0">Sort by:</span>
        {SORT_OPTIONS.map((option) => (
          <button
            key={option.key}
            onClick={() => setSelectedSort(option.key)}
            className={clsx(
              'px-3 py-1 text-sm rounded transition-colors min-h-[44px] flex items-center whitespace-nowrap flex-shrink-0',
              selectedSort === option.key
                ? 'bg-primary/20 text-primary font-medium'
                : 'text-on-surface-variant hover:text-primary'
            )}
          >
            {option.label}
          </button>
        ))}
      </div>

      {/* Store grid */}
      <div className="p-6 sidebar-scroll">
        {isLoading ? (
          <div className="flex items-center justify-center py-12">
            <div className="text-center">
              <div className="w-8 h-8 border-2 border-primary border-t-transparent rounded-full animate-spin mx-auto" />
              <p className="mt-3 text-sm text-on-surface-variant">Loading coaches...</p>
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
              {searchQuery ? "Couldn't search coaches" : "Couldn't load the store"}
            </h3>
            <p className="text-sm text-on-surface-variant mt-1">
              {listError instanceof Error && listError.message
                ? listError.message
                : 'The server did not return a coach list.'}
            </p>
            <button
              onClick={handleRetryList}
              className="mt-4 px-4 py-2 bg-primary text-on-primary font-medium rounded-lg hover:bg-primary/80 transition-colors min-h-[44px]"
            >
              Try Again
            </button>
          </div>
        ) : coaches.length === 0 ? (
          <div className="text-center py-12">
            <svg
              className="w-12 h-12 text-on-surface-variant mx-auto mb-4"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
              aria-hidden="true"
            >
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M9.172 16.172a4 4 0 015.656 0M9 10h.01M15 10h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
            </svg>
            <h3 className="text-lg font-medium text-on-surface">
              {searchQuery ? 'No coaches found' : 'Store is empty'}
            </h3>
            <p className="text-sm text-on-surface-variant mt-1">
              {searchQuery
                ? `No coaches match "${searchQuery}"`
                : 'No published coaches available yet'}
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
                    <span className="text-sm text-on-surface-variant">Loading more...</span>
                  </div>
                ) : hasNextPage ? (
                  <span className="text-sm text-on-surface-variant">Scroll for more</span>
                ) : coaches.length > 0 ? (
                  <span className="text-sm text-on-surface-variant">You've seen all coaches</span>
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
  const categoryColors = COACH_CATEGORY_COLORS[coach.category] ?? 'bg-surface-container-high/20 text-on-surface-variant';

  return (
    <button
      onClick={onClick}
      className="text-left p-4 bg-surface-container-low border ghost-border rounded-xl hover:border-primary/40 hover:bg-surface-container hover:shadow-ambient transition-all duration-200 group"
    >
      {/* Header with category and install count */}
      <div className="flex items-center justify-between mb-2">
        <span className={clsx('px-2.5 py-0.5 text-xs font-medium rounded-full capitalize', categoryColors)}>
          {coach.category}
        </span>
        <span className="text-xs text-on-surface-variant">
          {coach.install_count} {coach.install_count === 1 ? 'user' : 'users'}
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
              className="px-2 py-0.5 text-xs bg-surface-container-high text-on-surface-variant rounded"
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
  actionError: string | null;
  onBack: () => void;
  onInstall: () => void;
  onRemove: () => void;
}

function CoachDetailView({
  coach,
  isLoading,
  isInstalled,
  isInstalling,
  successMessage,
  actionError,
  onBack,
  onInstall,
  onRemove,
}: CoachDetailViewProps) {
  if (isLoading) {
    return (
      <div className="h-full flex flex-col bg-surface">
        <div className="flex-1 flex items-center justify-center">
          <div className="text-center">
            <div className="w-8 h-8 border-2 border-primary border-t-transparent rounded-full animate-spin mx-auto" />
            <p className="mt-3 text-sm text-on-surface-variant">Loading coach details...</p>
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
            <p className="text-lg text-on-surface-variant mb-4">Coach not found</p>
            <button
              onClick={onBack}
              className="px-4 py-2 bg-primary text-on-primary rounded-lg hover:bg-primary/80 transition-colors"
            >
              Go Back
            </button>
          </div>
        </div>
      </div>
    );
  }

  const categoryColors = COACH_CATEGORY_COLORS[coach.category] ?? 'bg-surface-container-high/20 text-on-surface-variant';

  return (
    <div className="h-full flex flex-col bg-surface">
      {/* Header with back button */}
      <div className="p-4 border-b ghost-border flex items-center gap-3">
        <button
          onClick={onBack}
          title="Back to Store"
          aria-label="Back to Store"
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
            <span className={clsx('px-3 py-1 text-sm font-medium rounded-full capitalize', categoryColors)}>
              {coach.category}
            </span>
            <span className="text-sm text-on-surface-variant">
              {coach.install_count} {coach.install_count === 1 ? 'user' : 'users'}
            </span>
          </div>

          {/* Description */}
          {coach.description && (
            <p className="text-base text-on-surface-variant leading-relaxed">{coach.description}</p>
          )}

          {/* Tags */}
          {coach.tags.length > 0 && (
            <div>
              <h3 className="text-sm font-semibold text-on-surface-variant uppercase tracking-wide mb-2">Tags</h3>
              <div className="flex flex-wrap gap-2">
                {coach.tags.map((tag, index) => (
                  <span
                    key={index}
                    className="px-3 py-1 text-sm bg-surface-container-high text-on-surface-variant rounded-full border ghost-border"
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
              <h3 className="text-sm font-semibold text-on-surface-variant uppercase tracking-wide mb-2">Sample Prompts</h3>
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
            <h3 className="text-sm font-semibold text-on-surface-variant uppercase tracking-wide mb-2">System Prompt</h3>
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
            <h3 className="text-sm font-semibold text-on-surface-variant uppercase tracking-wide mb-2">Details</h3>
            <div className="bg-surface-container-low border ghost-border rounded-lg overflow-hidden">
              <div className="flex justify-between items-center px-4 py-3 border-b ghost-border">
                <span className="text-sm text-on-surface-variant">Token Count</span>
                <span className="text-sm text-on-surface font-medium">{coach.token_count.toLocaleString()}</span>
              </div>
              {coach.published_at && (
                <div className="flex justify-between items-center px-4 py-3">
                  <span className="text-sm text-on-surface-variant">Published</span>
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

          {/* Success message. The coach now sits in "Your coaches" at the top
              of this same surface, so there is nowhere else to send the athlete. */}
          {successMessage && (
            <div className="p-4 bg-success/20 border border-success/30 rounded-lg">
              <p className="text-sm text-success">{successMessage}</p>
            </div>
          )}

          {/* Bottom spacer for fixed button */}
          <div className="h-20" />
        </div>
      </div>

      {/* Fixed action button at bottom */}
      <div className="p-4 border-t ghost-border bg-surface">
        {isInstalled ? (
          <button
            onClick={onRemove}
            disabled={isInstalling}
            className="w-full py-3 px-4 bg-surface-container-high border ghost-border rounded-lg text-on-surface font-medium hover:bg-surface-container-highest transition-colors disabled:opacity-50 disabled:cursor-not-allowed flex items-center justify-center gap-2"
          >
            {isInstalling ? (
              <div className="w-5 h-5 border-2 border-white border-t-transparent rounded-full animate-spin" />
            ) : (
              <>
                <Trash2 className="w-4 h-4" />
                Remove
              </>
            )}
          </button>
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
                Add Coach
              </>
            )}
          </button>
        )}
      </div>
    </div>
  );
}
