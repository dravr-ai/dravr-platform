// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A coach's Home on the phone — "Mes athlètes": À traiter, the team list and the season, behind the switch to their own training
// ABOUTME: One roster read feeds the three views; every figure states its scope, form is the server's band, and a non-sharer shows status only

import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { ActivityIndicator, RefreshControl, ScrollView, Text, View } from 'react-native';
import { Stack, useFocusEffect, useRouter } from 'expo-router';
import { COACH_SCOPE_KEYS, countKey } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { formatInstant, localToday } from '@pierre/ui-logic';
import { BrandLockup, EmptyState, TextTabs } from '../../components/ui';
import { useThemeColors } from '../../constants/theme';
import { useCoachRoster } from '../../hooks/useCoachRoster';
import { coachAthleteHref } from '../../navigation/routes';
import { CoachQueueView } from './CoachQueueView';
import { CoachRosterList } from './CoachRosterList';
import { CoachSeasonList } from './CoachSeasonList';
import { useCoachDestinations } from './useCoachDestinations';
import { useTreatment } from './useTreatment';

/** The three views behind the tabs under the switch. */
export type CoachView = 'queue' | 'roster' | 'season';

interface CoachHomeScreenProps {
  /** The "Mes athlètes / Mon entraînement" switch, drawn first on the page. */
  scopeSwitch: React.ReactNode;
}

const VIEW_LABEL_KEY: Record<CoachView, string> = {
  queue: 'coachHome.views.queue',
  roster: 'coachHome.views.roster',
  season: 'coachHome.views.season',
};
const VIEWS: readonly CoachView[] = ['queue', 'roster', 'season'];

export function CoachHomeScreen({ scopeSwitch }: CoachHomeScreenProps) {
  const { t, language } = useTranslation();
  const colors = useThemeColors();
  const router = useRouter();
  const scrollRef = useRef<ScrollView>(null);
  const { roster, isLoading, isError, readAt, refetch } = useCoachRoster(true);
  const destinations = useCoachDestinations();
  const { undo } = useTreatment();
  const [view, setView] = useState<CoachView>('queue');
  const [refreshing, setRefreshing] = useState(false);
  const today = useMemo(() => localToday(), []);

  // A return from a thread or a detail reads the roster again, as Home reads
  // its own; the first focus is the mount's own read. The hook's `refetch` is
  // a new function each render, and the router runs a focus callback again
  // whenever its identity changes while focused, so the callback holds it in
  // a ref and keeps its own identity: one that changed would read the roster
  // on every render.
  const refetchRef = useRef(refetch);
  useEffect(() => {
    refetchRef.current = refetch;
  });
  const focusedOnce = useRef(false);
  useFocusEffect(
    useCallback(() => {
      if (!focusedOnce.current) {
        focusedOnce.current = true;
        return;
      }
      void refetchRef.current();
    }, []),
  );
  const onRefresh = useCallback(() => {
    setRefreshing(true);
    void refetch().finally(() => setRefreshing(false));
  }, [refetch]);

  const scrollToTop = useCallback(() => scrollRef.current?.scrollTo({ y: 0, animated: true }), []);
  const open = useCallback((athleteId: string) => router.push(coachAthleteHref(athleteId)), [router]);

  const scopeLine = useMemo(() => {
    if (roster === undefined) return '';
    const { visible_members: visible, roster_members: total } = roster.scope;
    const line = t(countKey(visible, language, COACH_SCOPE_KEYS), { visible, total });
    if (readAt <= 0) return line;
    const time = formatInstant(new Date(readAt).toISOString(), language, { hour: '2-digit', minute: '2-digit', hourCycle: 'h23' });
    return `${line} · ${t('coachHome.readAt', { time })}`;
  }, [roster, readAt, language, t]);

  let body: React.ReactNode;
  if (isLoading) {
    body = (
      <View className="items-center py-10" accessibilityLabel={t('coachHome.loading')} testID="coach-home-loading">
        <ActivityIndicator color={colors.tokens.primary} />
      </View>
    );
  } else if (isError || roster === undefined) {
    body = (
      <EmptyState testID="coach-home-failed" action={{ label: t('common.retry'), onPress: refetch, testID: 'coach-home-retry' }}>
        {t('coachHome.loadFailed')}
      </EmptyState>
    );
  } else if (roster.athletes.length === 0) {
    body = <EmptyState testID="coach-home-empty">{t('coachHome.noAthletes')}</EmptyState>;
  } else if (view === 'queue') {
    body = <CoachQueueView athletes={roster.athletes} onOpen={open} onSeeAll={() => setView('roster')} undo={undo} />;
  } else if (view === 'roster') {
    body = (
      <CoachRosterList athletes={roster.athletes} groups={roster.groups} today={today} onOpen={open} destinations={destinations} />
    );
  } else {
    body = <CoachSeasonList athletes={roster.athletes} today={today} onOpen={open} />;
  }

  return (
    <View className="flex-1 bg-background-primary" testID="coach-home">
      <Stack.Screen
        options={{
          headerTitle: () => <BrandLockup accessibilityLabel={t('nav.home')} onPress={scrollToTop} testID="home-title" />,
        }}
      />
      <ScrollView
        ref={scrollRef}
        // The system header and tab bar inset the page themselves.
        contentInsetAdjustmentBehavior="automatic"
        contentContainerClassName="pb-8"
        refreshControl={<RefreshControl refreshing={refreshing} onRefresh={onRefresh} tintColor={colors.tokens.primary} />}
        testID="coach-home-scroll"
      >
        <View className="px-4 pb-2 pt-2">{scopeSwitch}</View>
        <TextTabs
          items={VIEWS.map((id) => ({ key: id, label: t(VIEW_LABEL_KEY[id]) }))}
          value={view}
          onChange={(key) => setView(key as CoachView)}
          testID="coach-views"
        />
        {scopeLine !== '' && (
          <Text className="px-4 pt-2 text-xs text-text-secondary" testID="coach-scope">
            {scopeLine}
          </Text>
        )}
        {body}
      </ScrollView>
    </View>
  );
}
