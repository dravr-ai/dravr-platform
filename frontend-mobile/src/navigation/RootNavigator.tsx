// ABOUTME: Root navigation component handling auth state
// ABOUTME: Shows AuthStack for unauthenticated users, AppNavigator for authenticated users

import React, { useCallback } from 'react';
import { View, ActivityIndicator } from 'react-native';
import { NavigationContainer } from '@react-navigation/native';
import * as SplashScreen from 'expo-splash-screen';
import { useAuth } from '../contexts/AuthContext';
import { AuthStack } from './AuthStack';
import { AppNavigator } from './AppNavigator';
import { colors } from '../constants/theme';

// Keep the splash screen visible while auth state resolves
SplashScreen.preventAutoHideAsync();

export function RootNavigator() {
  const { isAuthenticated, isLoading, user } = useAuth();

  // Hide splash screen once auth check is done
  const onLayoutReady = useCallback(() => {
    if (!isLoading) {
      SplashScreen.hideAsync();
    }
  }, [isLoading]);

  // Show loading screen while checking auth state
  if (isLoading) {
    return (
      <View className="flex-1 items-center justify-center bg-background-primary">
        <ActivityIndicator size="large" color={colors.primary[500]} />
      </View>
    );
  }

  // If user is pending approval, show auth stack (will navigate to pending screen)
  const showAuthStack = !isAuthenticated || user?.user_status === 'pending';

  return (
    <NavigationContainer onReady={onLayoutReady}>
      {showAuthStack ? <AuthStack /> : <AppNavigator />}
    </NavigationContainer>
  );
}
