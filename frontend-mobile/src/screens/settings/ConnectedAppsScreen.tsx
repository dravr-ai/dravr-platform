// ABOUTME: Connected apps screen — lists external MCP OAuth clients (e.g. Claude Desktop) the user approved on the consent screen
// ABOUTME: Each row shows the client, its scope + grant date, with a per-row Revoke action (React Query + confirm Alert)
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React from 'react';
import {
  View,
  Text,
  FlatList,
  TouchableOpacity,
  ActivityIndicator,
  Alert,
  RefreshControl,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useRouter } from 'expo-router';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { Feather } from '@expo/vector-icons';
import type { OAuthGrant } from '@pierre/shared-types';
import { spacing, useThemeColors } from '../../constants/theme';
import { oauthApi } from '../../services/api';

const CONNECTED_APPS_QUERY_KEY = ['oauth', 'connected-apps'] as const;

function formatGrantedDate(iso: string): string {
  try {
    return new Date(iso).toLocaleDateString();
  } catch {
    return iso;
  }
}

export function ConnectedAppsScreen(): React.JSX.Element {
  const router = useRouter();
  const colors = useThemeColors();
  const queryClient = useQueryClient();

  const { data, isLoading, isError, error, refetch, isFetching } = useQuery({
    queryKey: CONNECTED_APPS_QUERY_KEY,
    queryFn: () => oauthApi.listConnectedApps(),
  });

  const revokeMutation = useMutation({
    mutationFn: (grantId: string) => oauthApi.revokeConnectedApp(grantId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: CONNECTED_APPS_QUERY_KEY });
    },
    onError: (err: unknown) => {
      const msg = err instanceof Error ? err.message : String(err);
      Alert.alert('Could not revoke app', msg);
    },
  });

  const grants = data ?? [];

  const handleRevoke = (grant: OAuthGrant): void => {
    Alert.alert(
      'Revoke access?',
      `"${grant.client_id}" will lose access to your data and must ask for your approval again the next time it connects. This cannot be undone.`,
      [
        { text: 'Cancel', style: 'cancel' },
        {
          text: 'Revoke',
          style: 'destructive',
          onPress: () => revokeMutation.mutate(grant.id),
        },
      ],
    );
  };

  const renderItem = ({ item }: { item: OAuthGrant }): React.JSX.Element => {
    const revoking = revokeMutation.isPending && revokeMutation.variables === item.id;
    return (
      <View
        testID={`connected-app-${item.id}`}
        className="flex-row items-start justify-between gap-3 rounded-2xl border border-white/10 bg-white/5 p-4 mb-3"
      >
        <View className="flex-1">
          <Text
            className="text-base font-semibold text-text-primary"
            numberOfLines={1}
          >
            {item.client_id}
          </Text>
          <Text className="text-sm text-text-secondary mt-1" numberOfLines={2}>
            {item.scope}
          </Text>
          <Text className="text-xs text-text-tertiary mt-1">
            Connected {formatGrantedDate(item.granted_at)}
          </Text>
        </View>
        <TouchableOpacity
          accessibilityRole="button"
          accessibilityLabel={`Revoke ${item.client_id}`}
          onPress={() => handleRevoke(item)}
          disabled={revoking}
          className="flex-row items-center gap-1.5 rounded-lg bg-red-500/10 px-3 py-2"
          testID={`revoke-${item.id}`}
        >
          {revoking ? (
            <ActivityIndicator size="small" color={colors.pierre.red} />
          ) : (
            <>
              <Feather name="trash-2" size={16} color={colors.pierre.red} />
              <Text className="text-sm font-semibold text-red-400">Revoke</Text>
            </>
          )}
        </TouchableOpacity>
      </View>
    );
  };

  return (
    <SafeAreaView
      className="flex-1 bg-background-primary"
      testID="connected-apps-screen"
    >
      {/* Header */}
      <View className="flex-row items-center px-4 py-3 border-b border-white/10">
        <TouchableOpacity
          className="p-2 mr-2"
          onPress={() => router.back()}
          testID="back-button"
        >
          <Feather name="arrow-left" size={24} color={colors.text.primary} />
        </TouchableOpacity>
        <Text className="flex-1 text-lg font-bold text-text-primary">
          Connected apps
        </Text>
      </View>

      {isLoading ? (
        <View className="flex-1 items-center justify-center">
          <ActivityIndicator color={colors.text.primary} />
        </View>
      ) : isError ? (
        <View className="flex-1 items-center justify-center px-6">
          <Text className="text-red-400 text-center">
            Failed to load connected apps:{' '}
            {error instanceof Error ? error.message : String(error)}
          </Text>
        </View>
      ) : (
        <FlatList
          data={grants}
          keyExtractor={(item) => item.id}
          renderItem={renderItem}
          contentContainerStyle={{
            padding: spacing.md,
            paddingBottom: spacing.xl,
            flexGrow: 1,
          }}
          refreshControl={
            <RefreshControl
              refreshing={isFetching}
              onRefresh={() => {
                refetch();
              }}
              tintColor={colors.text.primary}
            />
          }
          ListHeaderComponent={
            <Text className="text-text-secondary text-sm leading-relaxed mb-4">
              Apps you approved to access your Dravr data through the OAuth
              consent screen (for example Claude Desktop). Revoke any one to cut
              off its access — it will have to ask for your approval again.
            </Text>
          }
          ListEmptyComponent={
            <View className="flex-1 items-center justify-center py-16">
              <Feather name="link-2" size={48} color={colors.text.tertiary} />
              <Text className="text-text-secondary mt-3 text-center">
                No connected apps
              </Text>
              <Text className="text-text-tertiary text-xs mt-1 text-center px-6">
                When you approve an external app on the OAuth consent screen, it
                will appear here.
              </Text>
            </View>
          }
        />
      )}
    </SafeAreaView>
  );
}
