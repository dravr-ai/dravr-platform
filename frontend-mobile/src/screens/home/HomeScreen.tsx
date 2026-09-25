// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Home tab — today's session from the plan, the week around it, and the latest activities with their routes
// ABOUTME: Where the app lands after sign-in; every day and activity on it opens a new chat with a drafted question

import React, { useCallback, useEffect, useRef, useState } from 'react';
import { RefreshControl, ScrollView, View } from 'react-native';
import { Stack, useFocusEffect, useRouter } from 'expo-router';
import { useTranslation } from '@pierre/i18n';
import { BrandLockup } from '../../components/ui';
import { useThemeColors } from '../../constants/theme';
import { useProviderConnected, useRecentActivities, useTrainingPlan } from '../../hooks/useHome';
import { CONNECTIONS_ROUTE, threadHref } from '../../navigation/routes';
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

  // The activity list carries no provider flag, so an empty list asks the
  // provider status whether there is anything to read from — and only then:
  // a list with rows has its answer already.
  const listIsEmpty = recent.hasData && recent.activities.length === 0;
  const provider = useProviderConnected(listIsEmpty);

  // The queries fetch on mount; a later focus — back from a chat that built a
  // plan, from Connections, from a notification — reads them again so Home
  // shows what changed while it was out of sight: the provider status too
  // while the list is empty, since Connections is where the empty state
  // sends the athlete. The route answers are left alone: a completed
  // activity's route does not change.
  //
  // Whether the list is empty is read through a ref: a focus callback whose
  // identity changed would be run again by the router while the tab is
  // focused, and the list turning out empty on the first load would then
  // read everything a second time.
  const focusedOnce = useRef(false);
  const listIsEmptyNow = useRef(listIsEmpty);
  useEffect(() => {
    listIsEmptyNow.current = listIsEmpty;
  }, [listIsEmpty]);
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
      if (listIsEmptyNow.current) {
        void refetchProvider();
      }
    }, [refetchPlan, refetchRecent, refetchProvider]),
  );

  const onRefresh = useCallback(() => {
    setRefreshing(true);
    void Promise.allSettled([refetchPlan(), refetchRecent()]).finally(() => setRefreshing(false));
  }, [refetchPlan, refetchRecent]);

  // Every tap on Home asks the agent about what was tapped, in a fresh
  // thread, with the question in the composer and the send left to the athlete.
  const openDraft = useCallback(
    (draft: string) => router.push(threadHref(undefined, { draft })),
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
          stale={recent.stale}
          staleRefetch={recent.staleRefetch}
          asOf={recent.asOf}
          onRetry={() => void refetchRecent()}
          providerConnected={provider.connected}
          onConnect={() => router.push(CONNECTIONS_ROUTE)}
          openDraft={openDraft}
        />
      </ScrollView>
    </View>
  );
}
