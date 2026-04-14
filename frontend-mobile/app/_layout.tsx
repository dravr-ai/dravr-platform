// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Root layout for Expo Router providing the app-wide provider stack
// ABOUTME: Wraps all routes with Auth, Query, WebSocket providers and handles auth gating

import '../global.css';
import React from 'react';
import { View, ActivityIndicator, LogBox, useColorScheme } from 'react-native';
import { Slot, useSegments, useRouter } from 'expo-router';
import { StatusBar } from 'expo-status-bar';
import * as SplashScreen from 'expo-splash-screen';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import { GestureHandlerRootView } from 'react-native-gesture-handler';
import Toast from 'react-native-toast-message';
import { toastConfig } from '../src/config/toast';
import { ErrorBoundary } from '../src/components/ErrorBoundary';
import { AuthProvider, useAuth } from '../src/contexts/AuthContext';
import { QueryProvider } from '../src/providers/QueryProvider';
import { WebSocketProvider } from '../src/contexts/WebSocketContext';
import { BOREAL_LIGHT, BOREAL_DARK } from '../src/constants/theme';

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
  const scheme = useColorScheme();
  const tokens = scheme === 'dark' ? BOREAL_DARK : BOREAL_LIGHT;

  // Hide the native splash screen once auth state is resolved
  React.useEffect(() => {
    if (!isLoading) {
      SplashScreen.hideAsync();
    }
  }, [isLoading]);

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

  if (isLoading) {
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

export default function RootLayout() {
  // `auto` lets StatusBar follow the system color scheme — light icons on
  // dark surfaces, dark icons on light surfaces.
  return (
    <GestureHandlerRootView style={{ flex: 1 }}>
      <ErrorBoundary>
        <SafeAreaProvider>
          <AuthProvider>
            <QueryProvider>
              <WebSocketProvider>
                <StatusBar style="auto" />
                <RootLayoutNav />
                <Toast config={toastConfig} />
              </WebSocketProvider>
            </QueryProvider>
          </AuthProvider>
        </SafeAreaProvider>
      </ErrorBoundary>
    </GestureHandlerRootView>
  );
}
