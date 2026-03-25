// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Group list screen showing coaching groups the user belongs to
// ABOUTME: Pull-to-refresh, empty state, and FAB for creating new groups

import React, { useCallback } from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  ActivityIndicator,
  RefreshControl,
} from 'react-native';
import { FlashList } from '@shopify/flash-list';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useRouter } from 'expo-router';
import { Feather } from '@expo/vector-icons';
import { colors, spacing, buttonGlow } from '../../constants/theme';
import { useMyGroups } from '../../hooks/useGroups';
import { GroupCard } from '../../components/groups/GroupCard';
import type { GroupSummary } from '../../types';

export function GroupListScreen() {
  const router = useRouter();
  const {
    groups,
    isLoading,
    isRefetching,
    isError,
    error,
    refetch,
  } = useMyGroups();

  const handleGroupPress = useCallback(
    (group: GroupSummary) => {
      router.push({
        pathname: '/(app)/(tabs)/(groups)/[groupId]',
        params: { groupId: group.id },
      });
    },
    [router],
  );

  const handleCreateGroup = useCallback(() => {
    router.push('/(app)/(tabs)/(groups)/create');
  }, [router]);

  const handleJoinGroup = useCallback(() => {
    router.push('/(app)/(tabs)/(groups)/join');
  }, [router]);

  const renderGroup = useCallback(
    ({ item, index }: { item: GroupSummary; index: number }) => (
      <GroupCard
        group={item}
        onPress={handleGroupPress}
        testID={`group-card-${index}`}
      />
    ),
    [handleGroupPress],
  );

  const renderEmptyState = () => (
    <View className="flex-1 justify-center items-center p-6">
      <View
        className="w-24 h-24 rounded-full items-center justify-center mb-2"
        style={{
          backgroundColor: 'rgba(139, 92, 246, 0.1)',
          shadowColor: colors.pierre.violet,
          shadowOffset: { width: 0, height: 0 },
          shadowOpacity: 0.3,
          shadowRadius: 20,
        }}
      >
        <Feather name="users" size={48} color={colors.pierre.violet} />
      </View>
      <Text className="text-text-primary text-xl font-bold mt-4">
        No Groups Yet
      </Text>
      <Text className="text-text-secondary text-base text-center mt-2 mb-6">
        Join a coaching group or create your own to train with others
      </Text>
      <View className="flex-row gap-3">
        <TouchableOpacity
          className="flex-row items-center px-5 py-3 rounded-xl gap-2 border border-border-strong"
          onPress={handleJoinGroup}
          testID="join-group-empty-button"
        >
          <Feather name="log-in" size={18} color={colors.text.primary} />
          <Text className="text-text-primary text-base font-semibold">
            Join Group
          </Text>
        </TouchableOpacity>
        <TouchableOpacity
          className="flex-row items-center px-5 py-3 rounded-xl gap-2"
          style={{
            backgroundColor: colors.pierre.violet,
            ...buttonGlow,
          }}
          onPress={handleCreateGroup}
          testID="create-group-empty-button"
        >
          <Feather name="plus" size={18} color="#FFFFFF" />
          <Text className="text-white text-base font-semibold">Create</Text>
        </TouchableOpacity>
      </View>
    </View>
  );

  if (isLoading && groups.length === 0) {
    return (
      <SafeAreaView className="flex-1 bg-background-primary" testID="group-list-screen">
        <View className="flex-1 justify-center items-center">
          <ActivityIndicator size="large" color={colors.pierre.violet} />
          <Text className="text-text-secondary mt-4">Loading groups...</Text>
        </View>
      </SafeAreaView>
    );
  }

  return (
    <SafeAreaView className="flex-1 bg-background-primary" testID="group-list-screen">
      {/* Header */}
      <View className="flex-row items-center px-4 py-4 border-b border-border-subtle">
        <View className="w-10" />
        <Text className="flex-1 text-xl font-bold text-text-primary text-center">
          Groups
        </Text>
        <View className="flex-row gap-2">
          <TouchableOpacity
            className="p-2"
            onPress={handleJoinGroup}
            testID="join-group-header-button"
          >
            <Feather name="log-in" size={22} color={colors.text.primary} />
          </TouchableOpacity>
          <TouchableOpacity
            className="p-2"
            onPress={handleCreateGroup}
            testID="create-group-header-button"
          >
            <Feather name="plus-circle" size={22} color={colors.pierre.violet} />
          </TouchableOpacity>
        </View>
      </View>

      {/* Error Display */}
      {isError && (
        <View className="mx-4 mt-2 p-3 bg-error/10 border border-error/30 rounded-lg flex-row items-center justify-between">
          <Text className="flex-1 text-error text-sm mr-3">
            {error instanceof Error ? error.message : 'Failed to load groups'}
          </Text>
          <TouchableOpacity
            className="px-3 py-1.5 bg-error/20 rounded-md"
            onPress={() => refetch()}
          >
            <Text className="text-error text-sm font-semibold">Retry</Text>
          </TouchableOpacity>
        </View>
      )}

      {/* Group List */}
      <FlashList
        testID="group-list"
        data={groups}
        keyExtractor={(item) => item.id}
        renderItem={renderGroup}
        ListEmptyComponent={renderEmptyState}
        contentContainerStyle={
          groups.length === 0
            ? { flexGrow: 1 }
            : { paddingVertical: spacing.sm }
        }
        refreshControl={
          <RefreshControl
            refreshing={isRefetching}
            onRefresh={() => refetch()}
            tintColor={colors.pierre.violet}
          />
        }
      />
    </SafeAreaView>
  );
}
