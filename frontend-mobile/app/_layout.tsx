// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Root layout for Expo Router providing the app-wide provider stack
// ABOUTME: Wraps all routes with Auth and Query providers and handles auth gating

import '../global.css';
import React from 'react';
import { View, ActivityIndicator, LogBox } from 'react-native';
import { Slot, useSegments, useRouter, useNavigationContainerRef } from 'expo-router';
import { StatusBar } from 'expo-status-bar';
import * as SplashScreen from 'expo-splash-screen';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import { GestureHandlerRootView } from 'react-native-gesture-handler';
import Toast from 'react-native-toast-message';
import {
  useFonts as useSpaceGrotesk,
  SpaceGrotesk_400Regular,
  SpaceGrotesk_500Medium,
  SpaceGrotesk_600SemiBold,
  SpaceGrotesk_700Bold,
} from '@expo-google-fonts/space-grotesk';
import {
  PlusJakartaSans_400Regular,
  PlusJakartaSans_500Medium,
  PlusJakartaSans_600SemiBold,
  PlusJakartaSans_700Bold,
} from '@expo-google-fonts/plus-jakarta-sans';
import {
  Inter_400Regular,
  Inter_500Medium,
  Inter_600SemiBold,
} from '@expo-google-fonts/inter';
import { toastConfig } from '../src/config/toast';
import { ErrorBoundary } from '../src/components/ErrorBoundary';
import { AuthProvider, useAuth } from '../src/contexts/AuthContext';
import { QueryProvider } from '../src/providers/QueryProvider';
import { ThemeProvider, useTheme } from '../src/contexts/ThemeContext';
import { useOnboardingStatus } from '../src/hooks/useOnboardingStatus';
import { useCoachProposalSeen } from '../src/hooks/useCoachProposalSeen';
import { trackMobile } from '../src/services/analytics';

LogBox.ignoreLogs([
  'Failed to send message:',
  'Failed to load conversations:',
  'Failed to load messages:',
  'Failed to create conversation:',
  'AxiosError',
]);

SplashScreen.preventAutoHideAsync();

function RootLayoutNav() {
  const { isAuthenticated, isLoading, user } = useAuth();
  const segments = useSegments();
  const router = useRouter();
  const navigationRef = useNavigationContainerRef();
  // ThemeProvider resolves the user's appearance preference (System / Light /
  // Dark, default = Dark) from AsyncStorage and pushes it to NativeWind so
  // every Tailwind class flips automatically. `tokens` is the live BOREAL_*
  // palette for inline JS styles below.
  const { colors: themeColors } = useTheme();
  const tokens = themeColors.tokens;

  // Load the Boreal Editorial typography stack. Keys match the font family
  // names declared in tailwind.config.js (`SpaceGrotesk`, `PlusJakartaSans`,
  // `Inter`) so NativeWind className props resolve automatically once loaded.
  const [fontsLoaded] = useSpaceGrotesk({
    SpaceGrotesk: SpaceGrotesk_400Regular,
    SpaceGrotesk_Medium: SpaceGrotesk_500Medium,
    SpaceGrotesk_SemiBold: SpaceGrotesk_600SemiBold,
    SpaceGrotesk_Bold: SpaceGrotesk_700Bold,
    PlusJakartaSans: PlusJakartaSans_400Regular,
    PlusJakartaSans_Medium: PlusJakartaSans_500Medium,
    PlusJakartaSans_SemiBold: PlusJakartaSans_600SemiBold,
    PlusJakartaSans_Bold: PlusJakartaSans_700Bold,
    Inter: Inter_400Regular,
    Inter_Medium: Inter_500Medium,
    Inter_SemiBold: Inter_600SemiBold,
  });

  // Hide the native splash screen once fonts + auth are both ready.
  React.useEffect(() => {
    if (!isLoading && fontsLoaded) {
      SplashScreen.hideAsync();
    }
  }, [isLoading, fontsLoaded]);

  // Boot/teardown PostHog in lockstep with analytics_consent. Default
  // is opt-out so this is a no-op until the user enables it under
  // Privacy & Data on the web settings (mobile reads the same flag).
  React.useEffect(() => {
    let cancelled = false;
    void (async () => {
      const mod = await import('../src/services/analytics');
      if (cancelled) return;
      if (user && user.analytics_consent === true) {
        await mod.bootMobileAnalytics(user.id, true);
      } else {
        mod.shutdownMobileAnalytics();
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [user?.id, user?.analytics_consent]);

  // Emit a screen_view whenever the focused route changes. We read the route
  // name off the navigation container's state listener (the Expo Router
  // equivalent of NavigationContainer's onStateChange) and de-dupe so a
  // re-render that leaves the active screen unchanged doesn't double-fire.
  // trackMobile() is a no-op until consent boots PostHog, so this is safe to
  // wire unconditionally.
  React.useEffect(() => {
    let lastScreen: string | null = null;
    const emit = (): void => {
      const screen = navigationRef.getCurrentRoute()?.name;
      if (!screen || screen === lastScreen) return;
      lastScreen = screen;
      trackMobile({ name: 'screen_view', props: { screen } });
    };
    const unsubscribe = navigationRef.addListener('state', emit);
    // Fire once for the initial route the container lands on.
    if (navigationRef.isReady()) emit();
    return unsubscribe;
  }, [navigationRef]);

  // Onboarding gate: refuse to land the user on the chat tabs until they
  // have at least one connected fitness provider. The same source of truth
  // (`provider_connections`) drives the backend's 403 on chat/coach/messaging,
  // so the gate and the redirect can never drift.
  //
  // Admins are exempt — their primary intent is administering, not chatting,
  // so a missing provider must not block them out of the app entirely. The
  // backend 403 still applies if an admin posts a chat without a provider.
  const isAdminRole = user?.role === 'admin' || user?.role === 'super_admin';
  const needsOnboardingFetch =
    isAuthenticated && user?.user_status === 'active' && !isAdminRole;
  const { data: onboardingStatus } = useOnboardingStatus(needsOnboardingFetch);
  // One-time coach-proposal step shown after the provider connection lands.
  const { seen: coachProposalSeen } = useCoachProposalSeen(user?.id);

  // Only divert to the coach proposal when the user connects their first
  // provider *in this session* (a `needs_provider_connection` true→false
  // transition), so an already-onboarded user simply opening the app — or an
  // E2E run authenticating a provider-connected fixture — is never intercepted.
  const [justOnboarded, setJustOnboarded] = React.useState(false);
  const prevNeedsProvider = React.useRef<boolean | undefined>(undefined);
  React.useEffect(() => {
    const now = onboardingStatus?.needs_provider_connection;
    if (prevNeedsProvider.current === true && now === false) {
      setJustOnboarded(true);
    }
    prevNeedsProvider.current = now;
  }, [onboardingStatus?.needs_provider_connection]);

  React.useEffect(() => {
    if (isLoading) return;

    const inAuthGroup = segments[0] === '(auth)';
    const inOnboardingGroup = segments[0] === '(onboarding)';
    const onCoachProposal =
      inOnboardingGroup && (segments as readonly string[])[1] === 'coach-proposal';
    const showAuth = !isAuthenticated || user?.user_status === 'pending';

    if (showAuth) {
      if (!inAuthGroup) {
        router.replace('/(auth)/login');
      }
      return;
    }

    // Authenticated + active. Land the user on the chat stack by default and
    // only divert to onboarding when the server explicitly confirms
    // `needs_provider_connection: true`. Holding routing on the in-flight
    // query produced a flicker / hung-spinner on slow networks.
    if (onboardingStatus?.needs_provider_connection === true) {
      if (!inOnboardingGroup) {
        router.replace('/(onboarding)/connect');
      }
      return;
    }

    // Provider connected: insert the one-time coach-proposal step before chat.
    // `seen === undefined` means the AsyncStorage read is still in flight — hold
    // routing rather than flash chat then bounce back to the proposal.
    if (
      needsOnboardingFetch &&
      justOnboarded &&
      onboardingStatus?.needs_provider_connection === false
    ) {
      if (coachProposalSeen === undefined) {
        return;
      }
      if (coachProposalSeen === false) {
        if (!onCoachProposal) {
          router.replace('/(onboarding)/coach-proposal');
        }
        return;
      }
    }

    if (inAuthGroup || inOnboardingGroup) {
      router.replace('/(app)/(tabs)/(chat)');
    }
  }, [
    isAuthenticated,
    isLoading,
    segments,
    router,
    user?.user_status,
    needsOnboardingFetch,
    justOnboarded,
    onboardingStatus?.needs_provider_connection,
    coachProposalSeen,
  ]);

  if (isLoading || !fontsLoaded) {
    return (
      <View style={{ flex: 1, alignItems: 'center', justifyContent: 'center', backgroundColor: tokens.surface }}>
        <ActivityIndicator size="large" color={tokens.primary} />
      </View>
    );
  }

  return (
    <View style={{ flex: 1, backgroundColor: tokens.surface }}>
      <Slot />
    </View>
  );
}

function RootShell() {
  const { scheme } = useTheme();
  return (
    <>
      <StatusBar style={scheme === 'dark' ? 'light' : 'dark'} />
      <RootLayoutNav />
      <Toast config={toastConfig} />
    </>
  );
}

export default function RootLayout() {
  // ThemeProvider sits above auth + query so its preference resolution runs
  // independent of the auth state. StatusBar appearance follows the resolved
  // scheme so the system clock/battery glyphs stay legible in both modes.
  return (
    <GestureHandlerRootView style={{ flex: 1 }}>
      <ErrorBoundary>
        <SafeAreaProvider>
          <ThemeProvider>
            <AuthProvider>
              <QueryProvider>
                <RootShell />
              </QueryProvider>
            </AuthProvider>
          </ThemeProvider>
        </SafeAreaProvider>
      </ErrorBoundary>
    </GestureHandlerRootView>
  );
}
