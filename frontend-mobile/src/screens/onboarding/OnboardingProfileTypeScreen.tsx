// ABOUTME: First-run onboarding step (mobile) — asks whether the user is an athlete or a coach
// ABOUTME: Mirrors the web OnboardingProfileType; "coach" sets coaching_persona=coach; advances via the shared flag cache

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useState } from 'react';
import { View, Text, ScrollView, Pressable, ActivityIndicator } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { Card } from '../../components/ui';
import { userApi } from '../../services/api';
import { useAuth } from '../../contexts/AuthContext';
import { useProfileTypeChosen } from '../../hooks/useProfileTypeChosen';

/**
 * Athlete-vs-coach onboarding step (mobile).
 *
 * Reached via RootLayoutNav for a fresh account before the connect-provider step.
 * Picking "I coach others" persists `coaching_persona=coach`; either choice marks
 * the profile-type step done (locally + on the server) and flips the shared
 * `useProfileTypeChosen` cache, which routes the user on.
 */
export function OnboardingProfileTypeScreen() {
  const { user } = useAuth();
  const { markChosen } = useProfileTypeChosen(user?.id);
  const [choosing, setChoosing] = useState<'athlete' | 'coach' | null>(null);

  const finish = async (persona: 'athlete' | 'coach') => {
    if (choosing) return;
    setChoosing(persona);
    if (persona === 'coach') {
      try {
        await userApi.setCoachingPersona('coach');
      } catch {
        // Non-fatal: the default Casual voice is a harmless start; changeable in Settings.
      }
    }
    userApi.setOnboardingStep('profile_type', 'complete').catch(() => {});
    await markChosen();
  };

  const heading = user?.display_name ? `Welcome, ${user.display_name}` : 'Welcome to Dravr';

  return (
    <Shell heading={heading}>
      <Text className="mt-3 text-sm text-on-surface-variant text-center">
        How will you use Dravr? This tailors the coaches we suggest. You can change it anytime in
        Settings.
      </Text>
      <View className="mt-6 gap-4">
        <ChoiceCard
          title="I'm an athlete"
          description="Track your own training and get coaching tuned to how you actually train."
          busy={choosing === 'athlete'}
          disabled={choosing !== null}
          onSelect={() => void finish('athlete')}
        />
        <ChoiceCard
          title="I coach others"
          description="Build plans and manage the athletes you coach. Unlocks coach-facing plan builders."
          busy={choosing === 'coach'}
          disabled={choosing !== null}
          onSelect={() => void finish('coach')}
        />
      </View>
    </Shell>
  );
}

function ChoiceCard({
  title,
  description,
  busy,
  disabled,
  onSelect,
}: {
  title: string;
  description: string;
  busy: boolean;
  disabled: boolean;
  onSelect: () => void;
}) {
  return (
    <Pressable
      onPress={onSelect}
      disabled={disabled}
      accessibilityRole="button"
      accessibilityLabel={title}
      className="rounded-2xl border border-outline-variant bg-surface-container-low px-4 py-4 active:opacity-80"
      style={disabled ? { opacity: 0.6 } : undefined}
    >
      <Text className="text-base font-semibold text-on-surface">{title}</Text>
      <Text className="mt-1 text-sm text-on-surface-variant">{description}</Text>
      {busy ? (
        <View className="mt-2 flex-row items-center gap-2">
          <ActivityIndicator size="small" />
          <Text className="text-xs text-on-surface-variant">Setting things up…</Text>
        </View>
      ) : null}
    </Pressable>
  );
}

function Shell({ heading, children }: { heading?: string; children: React.ReactNode }) {
  return (
    <SafeAreaView className="flex-1 bg-background-primary" testID="profile-type-screen">
      <ScrollView contentContainerClassName="px-5 py-8">
        <Card>
          <View className="px-2 py-2">
            {heading ? (
              <Text className="text-2xl font-bold text-on-surface text-center">{heading}</Text>
            ) : null}
            {children}
          </View>
        </Card>
      </ScrollView>
    </SafeAreaView>
  );
}
