// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The athlete's installed coaches pinned at the top of Discover, each with its @handle
// ABOUTME: Its own query over the coach list — never a re-rank of the catalogue page, which is graded per cursor page

import React, { useCallback } from 'react';
import { View, Text, TouchableOpacity, FlatList, ActivityIndicator } from 'react-native';
import { useQuery } from '@tanstack/react-query';
import { useFocusEffect, useRouter } from 'expo-router';
import { Feather } from '@expo/vector-icons';
import { QUERY_KEYS } from '@pierre/shared-constants';
import { spacing, useThemeColors } from '../../constants/theme';
import { coachesApi } from '../../services/api';
import { COACH_DETAIL_ROUTE, COACH_EDITOR_ROUTE, COACH_LIBRARY_ROUTE } from '../../navigation/routes';
import type { Coach } from '../../types';

/**
 * The coaches the athlete already has, above the catalogue.
 *
 * This is where the old Coaches tab's library went: the list itself is here,
 * with a handle on every coach that has one, and the full library — filters,
 * favourites, hidden coaches, import and export, create and edit — is one tap
 * further. The strip reads `GET /api/coaches` directly rather than marking
 * catalogue rows: `handle_browse` re-ranks each cursor page by claim-verdict
 * grade, so an installed coach on page three could never surface on page one.
 */
export function InstalledCoachesStrip() {
  const colors = useThemeColors();
  const router = useRouter();

  const { data, isLoading, isError, refetch } = useQuery({
    queryKey: QUERY_KEYS.coaches.list(),
    queryFn: () => coachesApi.list(),
    select: (response) => response.coaches,
    staleTime: 60_000,
  });

  // An install or uninstall happens on the detail screen beneath this one;
  // refetch on focus so the strip reflects it on the way back.
  useFocusEffect(
    useCallback(() => {
      void refetch();
    }, [refetch]),
  );

  const coaches: Coach[] = data ?? [];

  return (
    <View className="border-b border-border-default" testID="installed-coaches-strip">
      <View className="flex-row items-center px-4 pt-3 pb-1">
        <Text className="flex-1 text-sm font-semibold text-text-secondary uppercase tracking-wide">
          Installed{coaches.length > 0 ? ` · ${coaches.length}` : ''}
        </Text>
        <TouchableOpacity
          className="px-2 py-1"
          onPress={() => router.push(COACH_LIBRARY_ROUTE)}
          accessibilityRole="button"
          accessibilityLabel="Manage my coaches"
          testID="manage-coaches-button"
        >
          <Text className="text-sm font-medium" style={{ color: colors.pierre.violet }}>
            Manage
          </Text>
        </TouchableOpacity>
        <TouchableOpacity
          className="w-8 h-8 items-center justify-center rounded-full"
          onPress={() => router.push(COACH_EDITOR_ROUTE)}
          accessibilityRole="button"
          accessibilityLabel="Create a coach"
          testID="discover-create-coach-button"
        >
          <Feather name="plus" size={20} color={colors.pierre.violet} />
        </TouchableOpacity>
      </View>

      {isLoading ? (
        <View className="px-4 pb-3">
          <ActivityIndicator size="small" color={colors.pierre.violet} testID="installed-coaches-loading" />
        </View>
      ) : isError ? (
        <View className="px-4 pb-3 flex-row items-center">
          <Text className="flex-1 text-sm text-error" testID="installed-coaches-error">
            Could not load your coaches.
          </Text>
          <TouchableOpacity onPress={() => void refetch()} accessibilityRole="button">
            <Text className="text-sm font-semibold text-error">Retry</Text>
          </TouchableOpacity>
        </View>
      ) : coaches.length === 0 ? (
        <Text className="px-4 pb-3 text-sm text-text-tertiary" testID="installed-coaches-empty">
          No coaches installed yet — pick one below.
        </Text>
      ) : (
        <FlatList
          horizontal
          showsHorizontalScrollIndicator={false}
          data={coaches}
          keyExtractor={(coach) => coach.id}
          contentContainerStyle={{ paddingHorizontal: spacing.md, paddingBottom: spacing.sm, gap: spacing.sm }}
          testID="installed-coaches-list"
          renderItem={({ item }) => (
            <TouchableOpacity
              className="rounded-xl px-3 py-2 border border-border-default"
              style={{ backgroundColor: colors.background.elevated, maxWidth: 220 }}
              onPress={() =>
                router.push({ pathname: COACH_DETAIL_ROUTE, params: { coachId: item.id } })
              }
              accessibilityRole="button"
              accessibilityLabel={item.handle ? `${item.title}, @${item.handle}` : item.title}
              testID={`installed-coach-${item.id}`}
            >
              <Text className="text-sm font-semibold text-text-primary" numberOfLines={1}>
                {item.title}
              </Text>
              {item.handle ? (
                <Text
                  className="text-xs mt-0.5"
                  style={{ color: colors.pierre.violet }}
                  numberOfLines={1}
                  testID={`installed-coach-handle-${item.id}`}
                >
                  @{item.handle}
                </Text>
              ) : (
                <Text className="text-xs text-text-tertiary mt-0.5 capitalize" numberOfLines={1}>
                  {item.category}
                </Text>
              )}
            </TouchableOpacity>
          )}
        />
      )}
    </View>
  );
}
