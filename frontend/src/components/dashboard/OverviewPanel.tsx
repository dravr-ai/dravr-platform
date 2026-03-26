// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Overview panel for the admin dashboard with coaching-centric metrics
// ABOUTME: Fetches analytics overview, pending users, and store stats for the OverviewTab

import { useEffect } from 'react';
import { useQuery } from '@tanstack/react-query';
import { adminAnalyticsApi, adminApi } from '../../services/api';
import { useWebSocketContext } from '../../hooks/useWebSocketContext';
import { QUERY_KEYS } from '../../constants/queryKeys';
import type { User } from '../../types/api';
import type { AdminAnalyticsOverview } from '../../services/api/admin-analytics';
import OverviewTab from '../OverviewTab';

interface OverviewPanelProps {
  onNavigate?: (tab: string) => void;
}

export default function OverviewPanel({ onNavigate }: OverviewPanelProps) {
  const { lastMessage } = useWebSocketContext();

  const { data: overview, isLoading: overviewLoading, refetch: refetchOverview } = useQuery<AdminAnalyticsOverview>({
    queryKey: QUERY_KEYS.adminAnalytics.overview(),
    queryFn: () => adminAnalyticsApi.getOverview(),
  });

  const { data: pendingUsers = [] } = useQuery<User[]>({
    queryKey: QUERY_KEYS.adminUsers.pending(),
    queryFn: () => adminApi.getPendingUsers(),
    staleTime: 30_000,
    retry: false,
  });

  const { data: storeStats } = useQuery({
    queryKey: QUERY_KEYS.adminStore.stats(),
    queryFn: () => adminApi.getStoreStats(),
    staleTime: 30_000,
    retry: false,
  });

  // Refresh data when WebSocket updates are received
  useEffect(() => {
    if (lastMessage) {
      if (lastMessage.type === 'usage_update' || lastMessage.type === 'system_stats') {
        refetchOverview();
      }
    }
  }, [lastMessage, refetchOverview]);

  return (
    <OverviewTab
      overview={overview}
      overviewLoading={overviewLoading}
      pendingUsersCount={pendingUsers.length}
      pendingCoachReviews={storeStats?.pending_count ?? 0}
      onNavigate={onNavigate}
    />
  );
}
