// ABOUTME: Discover screen — the Agent Store catalogue to install agents from
// ABOUTME: A hairline list of DiscoverRows under one category text-tabs row; search and sort live in the native header

import React, { useState, useCallback, useRef, useEffect } from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  ActivityIndicator,
  RefreshControl,
} from 'react-native';
import { FlashList } from '@shopify/flash-list';
import { Stack, useFocusEffect, useRouter } from 'expo-router';
import { Feather } from '@expo/vector-icons';

import { spacing, useThemeColors } from '../../constants/theme';
import { storeApi } from '../../services/api';
import { useAuth } from '../../contexts/AuthContext';
import type { StoreAgent, AgentCategory } from '../../types';
import { useTranslation } from '@pierre/i18n';
import { COACH_CATEGORY_LABEL_KEY } from '@pierre/shared-constants';
import { TextTabs, HeaderActions, type TextTabItem } from '../../components/ui';
import { DiscoverRow } from './DiscoverRow';
import { presentMenu } from '../../utils/presentMenu';
import { describeApiError } from '@pierre/ui-logic';

// Category tabs. `key` is the value sent to the API and must stay English;
// the label is resolved at render, since module scope cannot hold a hook.
// The tabs and the row's glyph tint read the same shared table, so a
// category never shows one hue on the tab and another on the row.
const CATEGORY_TABS: Array<{ key: AgentCategory | 'all'; labelKey: string }> = [
  { key: 'all', labelKey: 'discover.filterAll' },
  { key: 'training', labelKey: COACH_CATEGORY_LABEL_KEY.training },
  { key: 'nutrition', labelKey: COACH_CATEGORY_LABEL_KEY.nutrition },
  { key: 'recovery', labelKey: COACH_CATEGORY_LABEL_KEY.recovery },
  { key: 'recipes', labelKey: COACH_CATEGORY_LABEL_KEY.recipes },
  { key: 'mobility', labelKey: COACH_CATEGORY_LABEL_KEY.mobility },
  { key: 'custom', labelKey: COACH_CATEGORY_LABEL_KEY.custom },
];

// Sort options — offered from the header's sliders menu, not an inline row.
type SortOption = 'newest' | 'popular' | 'title';
const SORT_OPTIONS: Array<{ key: SortOption; labelKey: string }> = [
  { key: 'popular', labelKey: 'discover.sortPopular' },
  { key: 'newest', labelKey: 'discover.sortNewest' },
  { key: 'title', labelKey: 'discover.sortAlpha' },
];

export function StoreScreen() {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const router = useRouter();
  const { isAuthenticated } = useAuth();
  const [coaches, setCoaches] = useState<StoreAgent[]>([]);
  const [selectedCategory, setSelectedCategory] = useState<AgentCategory | 'all'>('all');
  const [selectedSort, setSelectedSort] = useState<SortOption>('popular');
  const [searchQuery, setSearchQuery] = useState('');
  const [isLoading, setIsLoading] = useState(false);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [isLoadingMore, setIsLoadingMore] = useState(false);
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [hasMore, setHasMore] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const searchTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Cleanup search timer on unmount
  useEffect(() => {
    return () => {
      if (searchTimerRef.current) {
        clearTimeout(searchTimerRef.current);
      }
    };
  }, []);

  const loadCoaches = useCallback(async (isRefresh = false) => {
    if (!isAuthenticated) return;

    try {
      if (isRefresh) {
        setIsRefreshing(true);
      } else {
        setIsLoading(true);
      }
      setError(null);

      const response = await storeApi.browse({
        category: selectedCategory === 'all' ? undefined : selectedCategory,
        sort_by: selectedSort,
        limit: 20,
      });
      setCoaches(response.agents);
      setNextCursor(response.next_cursor ?? null);
      setHasMore(response.has_more ?? false);
    } catch (err) {
      const errorMessage = describeApiError(err, { t, fallbackKey: 'app.failedLoadAgents' });
      setError(errorMessage);
      console.error('Failed to load store coaches:', err);
    } finally {
      setIsLoading(false);
      setIsRefreshing(false);
    }
  }, [isAuthenticated, selectedCategory, selectedSort, t]);

  const loadMoreCoaches = useCallback(async () => {
    if (!isAuthenticated || !hasMore || isLoadingMore || !nextCursor) return;

    try {
      setIsLoadingMore(true);
      const response = await storeApi.browse({
        category: selectedCategory === 'all' ? undefined : selectedCategory,
        sort_by: selectedSort,
        limit: 20,
        cursor: nextCursor,
      });
      setCoaches(prev => [...prev, ...response.agents]);
      setNextCursor(response.next_cursor ?? null);
      setHasMore(response.has_more ?? false);
    } catch (err) {
      const errorMessage = describeApiError(err, { t, fallbackKey: 'app.failedLoadMoreAgents' });
      setError(errorMessage);
      console.error('Failed to load more coaches:', err);
    } finally {
      setIsLoadingMore(false);
    }
  }, [isAuthenticated, hasMore, isLoadingMore, nextCursor, selectedCategory, selectedSort, t]);

  const searchCoaches = useCallback(async (query: string) => {
    if (!isAuthenticated || !query.trim()) {
      loadCoaches();
      return;
    }

    try {
      setError(null);
      const response = await storeApi.search(query.trim(), 50);
      setCoaches(response.agents);
      setNextCursor(null);
      setHasMore(false);
    } catch (err) {
      const errorMessage = describeApiError(err, { t, fallbackKey: 'app.failedSearchAgents' });
      setError(errorMessage);
      console.error('Failed to search coaches:', err);
    }
  }, [isAuthenticated, loadCoaches, t]);

  // Reload when screen focuses or filters change
  useFocusEffect(
    useCallback(() => {
      if (searchQuery.trim()) {
        searchCoaches(searchQuery);
      } else {
        loadCoaches();
      }
    }, [loadCoaches, searchCoaches, searchQuery])
  );

  const handleSearch = (text: string) => {
    setSearchQuery(text);
    // Debounce search with cleanup
    if (searchTimerRef.current) {
      clearTimeout(searchTimerRef.current);
    }
    if (text.trim()) {
      searchTimerRef.current = setTimeout(() => {
        searchCoaches(text);
        searchTimerRef.current = null;
      }, 300);
    } else {
      loadCoaches();
    }
  };

  const navigateToCoachDetail = (coach: StoreAgent) => {
    router.push({ pathname: '/(app)/(tabs)/(discover)/[agentId]', params: { agentId: coach.id } });
  };

  const categoryTabItems: TextTabItem[] = CATEGORY_TABS.map(({ key, labelKey }) => ({
    key,
    label: t(labelKey),
  }));

  const openSortMenu = () => {
    presentMenu(
      SORT_OPTIONS.map(({ key, labelKey }) => ({ label: t(labelKey), onPress: () => setSelectedSort(key) })),
      { title: t('discover.sortByLabel'), cancelLabel: t('common.cancel') },
    );
  };

  const renderEmptyState = () => (
    // testID because the copy inside is translated: an e2e flow asserting the
    // English string passes or fails on the device's language rather than on
    // whether the list actually emptied. Every other element on this screen
    // carries one; this was the gap that made the search flow unassertable.
    <View className="flex-1 justify-center items-center py-16" testID="store-empty-state">
      <Text className="text-lg font-semibold text-text-primary mb-1">
        {searchQuery ? t('discover.noAgentsFound') : t('app.noAgentsAvailable')}
      </Text>
      <Text className="text-base text-text-secondary text-center">
        {searchQuery
          ? t('app.noAgentsMatch', { query: searchQuery })
          : t('discover.noPublishedAgents')}
      </Text>
    </View>
  );

  const renderError = () => (
    <View className="mx-4 my-2 p-3 bg-error/10 border border-error/30 rounded-lg flex-row items-center justify-between">
      <View className="flex-1 mr-3">
        <Text className="text-error text-sm font-medium">{error}</Text>
      </View>
      <TouchableOpacity
        className="px-3 py-1.5 bg-error/20 rounded-md"
        onPress={() => {
          setError(null);
          loadCoaches();
        }}
      >
        <Text className="text-error text-sm font-semibold">{t('common.retry')}</Text>
      </TouchableOpacity>
    </View>
  );

  // The search field is the system's, in the native header: under the large
  // title on iOS 18, in the bottom toolbar on iOS 26, in the app bar on
  // Android. The sliders button opens the sort menu that used to be an
  // always-visible chip row under its own band.
  const headerOptions = (
    <Stack.Screen
      options={{
        headerRight: () => (
          <HeaderActions>
            <TouchableOpacity
              className="w-10 h-10 items-center justify-center"
              onPress={openSortMenu}
              hitSlop={{ top: 8, bottom: 8, left: 8, right: 8 }}
              accessibilityRole="button"
              accessibilityLabel={t('discover.sortByLabel')}
              testID="discover-sort-button"
            >
              <Feather name="sliders" size={20} color={colors.text.secondary} />
            </TouchableOpacity>
          </HeaderActions>
        ),
        headerSearchBarOptions: {
          placeholder: t('discover.searchAgentsPlaceholder'),
          autoCapitalize: 'none',
          hideWhenScrolling: false,
          onChangeText: (event) => handleSearch(event.nativeEvent.text),
          onSearchButtonPress: () => searchCoaches(searchQuery),
        },
      }}
    />
  );

  if (isLoading && coaches.length === 0) {
    return (
      <View className="flex-1 bg-background-primary" testID="store-screen">
        {headerOptions}
        <View className="flex-1 justify-center items-center" testID="loading-indicator">
          <ActivityIndicator size="large" color={colors.tokens.primary} />
          <Text className="mt-3 text-text-secondary text-base">{t('discover.loadingAgents')}</Text>
        </View>
      </View>
    );
  }

  return (
    <View className="flex-1 bg-background-primary" testID="store-screen">
      {headerOptions}

      {/* Category, as one scrollable row of text tabs — the two bordered chip
          bands (category pills, sort chips) are gone. */}
      <TextTabs
        testID="discover-category-tabs"
        items={categoryTabItems}
        value={selectedCategory}
        onChange={(key) => setSelectedCategory(key as AgentCategory | 'all')}
      />

      {/* Error Display */}
      {error && renderError()}

      {/* Coach list, a hairline row per listing, with FlashList for optimized performance */}
      <FlashList
        testID="coach-list"
        data={coaches}
        keyExtractor={(item) => item.id}
        renderItem={({ item, index }) => (
          <DiscoverRow agent={item} index={index} onPress={navigateToCoachDetail} />
        )}
        // The system header and tab bar inset the list themselves.
        contentInsetAdjustmentBehavior="automatic"
        contentContainerStyle={{ paddingVertical: spacing.sm }}
        ListEmptyComponent={renderEmptyState}
        onEndReached={loadMoreCoaches}
        onEndReachedThreshold={0.5}
        ListFooterComponent={
          isLoadingMore ? (
            <View className="flex-row items-center justify-center py-4 gap-2">
              <ActivityIndicator size="small" color={colors.tokens.primary} />
              <Text className="text-sm text-text-secondary">{t('app.loadingMoreAgents')}</Text>
            </View>
          ) : null
        }
        refreshControl={
          <RefreshControl
            refreshing={isRefreshing}
            onRefresh={() => loadCoaches(true)}
            tintColor={colors.tokens.primary}
          />
        }
      />
    </View>
  );
}
