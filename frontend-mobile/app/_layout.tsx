// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Root layout for Expo Router providing the app-wide provider stack
// ABOUTME: Wraps all routes with Auth, Query, WebSocket providers and handles auth gating

import '../global.css';
import React from 'react';
import { View, ActivityIndicator, LogBox } from 'react-native';
import { Slot, useSegments, useRouter } from 'expo-router';
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
import { WebSocketProvider } from '../src/contexts/WebSocketContext';
import { ThemeProvider, useTheme } from '../src/contexts/ThemeContext';

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

  React.useEffect(() => {
    if (isLoading) return;

    const inAuthGroup = segments[0] === '(auth)';
    const showAuth = !isAuthenticated || user?.user_status === 'pending';

    if (showAuth && !inAuthGroup) {
      router.replace('/(auth)/login');
    } else if (!showAuth && inAuthGroup) {
      router.replace('/(app)/(tabs)/(chat)');
    }
  }, [isAuthenticated, isLoading, segments, router, user?.user_status]);

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
                <WebSocketProvider>
                  <RootShell />
                </WebSocketProvider>
              </QueryProvider>
            </AuthProvider>
          </ThemeProvider>
        </SafeAreaProvider>
      </ErrorBoundary>
    </GestureHandlerRootView>
  );
}
