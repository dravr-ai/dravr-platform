// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Home tab — today's session from the plan, the week around it, and the latest activities with their routes
// ABOUTME: Where the app lands after sign-in; a day on it opens a chat with a drafted question, an activity its own view

import React, { useCallback, useRef, useState } from 'react';
import { RefreshControl, ScrollView, View } from 'react-native';
import { Stack, useFocusEffect, useRouter } from 'expo-router';
import { useQueryClient } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { ActivityRouteResponse, HomeActivity } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import { BrandLockup } from '../../components/ui';
import { useThemeColors } from '../../constants/theme';
import { useProviderConnected, useRecentActivities, useTrainingPlan } from '../../hooks/useHome';
import { CONNECTIONS_ROUTE, activityHref, threadHref } from '../../navigation/routes';
import { HomePlan } from './HomePlan';
import { RecentActivities } from './RecentActivities';

export function HomeScreen() {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const router = useRouter();
  const scrollRef = useRef<ScrollView>(null);
  const plan = useTrainingPlan();
  const recent = useRecentActivities();
  const [refreshing, setRefreshing] = useState(false);

  // The activity list carries no provider flag, so the provider status says
  // whether there is anything to read from, and whether any connection still
  // syncs. Which ones have to be reconnected is the shell banner's to say,
  // from this same status; the section never repeats it.
  const provider = useProviderConnected();

  // The queries fetch on mount; a later focus — back from a chat that built a
  // plan, from Connections, from a notification — reads them again so Home
  // shows what changed while it was out of sight: the provider status too,
  // since Connections is where the connect and reconnect prompts send the
  // athlete. A drawn route is left alone — a completed activity's route does
  // not change — while a route answered without one is asked again on its
  // own (`routeAnswerStaleTime`), and a pull to refresh re-asks it at once.
  //
  // The three refetches keep their identity, and so does the focus callback:
  // one whose identity changed would be run again by the router while the
  // tab is focused, and would read everything a second time.
  const focusedOnce = useRef(false);
  const { refetch: refetchPlan } = plan;
  const { refetch: refetchRecent } = recent;
  const { refetch: refetchProvider } = provider;
  useFocusEffect(
    useCallback(() => {
      if (!focusedOnce.current) {
        focusedOnce.current = true;
        return;
      }
      void refetchPlan();
      void refetchRecent();
      void refetchProvider();
    }, [refetchPlan, refetchRecent, refetchProvider]),
  );

  const queryClient = useQueryClient();
  const onRefresh = useCallback(() => {
    setRefreshing(true);
    const undrawnRoutes = queryClient.invalidateQueries({
      queryKey: QUERY_KEYS.home.activityRoutes,
      predicate: (query) => (query.state.data as ActivityRouteResponse | undefined)?.route === null,
    });
    void Promise.allSettled([refetchPlan(), refetchRecent(), refetchProvider(), undrawnRoutes]).finally(() =>
      setRefreshing(false),
    );
  }, [queryClient, refetchPlan, refetchRecent, refetchProvider]);

  // A plan day asks the agent about it, in a fresh thread, with the question
  // in the composer and the send left to the athlete; an activity opens its
  // own view, where the questions about it are.
  const openDraft = useCallback(
    (draft: string) => router.push(threadHref(undefined, { draft })),
    [router],
  );
  const openActivity = useCallback(
    (activity: HomeActivity) => router.push(activityHref(activity.provider, activity.id)),
    [router],
  );

  const scrollToTop = useCallback(() => scrollRef.current?.scrollTo({ y: 0, animated: true }), []);

  return (
    <View className="flex-1 bg-background-primary" testID="home-screen">
      {/*
        The lockup is Home's title, as it is the chat tab's (DESIGN.md §5). It
        is a button on both: from the chat tab a press comes Home, and here,
        already Home, it goes back to the top of the page — what a press on the
        active tab does.
      */}
      <Stack.Screen
        options={{
          headerTitle: () => (
            <BrandLockup accessibilityLabel={t('nav.home')} onPress={scrollToTop} testID="home-title" />
          ),
        }}
      />
      <ScrollView
        ref={scrollRef}
        // The system header and tab bar inset the page themselves.
        contentInsetAdjustmentBehavior="automatic"
        contentContainerClassName="py-4 gap-8"
        refreshControl={
          <RefreshControl refreshing={refreshing} onRefresh={onRefresh} tintColor={colors.tokens.primary} />
        }
        testID="home-scroll"
      >
        <HomePlan
          response={plan.response}
          isError={plan.isError}
          onRetry={() => void refetchPlan()}
          openDraft={openDraft}
        />
        <RecentActivities
          activities={recent.activities}
          hasData={recent.hasData}
          isError={recent.isError}
          refreshing={recent.refreshing}
          asOf={recent.asOf}
          syncFailure={recent.syncFailure}
          onRetry={() => void refetchRecent()}
          onRetrySync={recent.retry}
          providerConnected={provider.connected}
          syncing={provider.syncing}
          onConnect={() => router.push(CONNECTIONS_ROUTE)}
          openActivity={openActivity}
        />
      </ScrollView>
    </View>
  );
}
