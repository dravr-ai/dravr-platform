// ABOUTME: Registers Expo push token with the backend on app start
// ABOUTME: Handles permission requests, token retrieval, and server-side device registration

import { useEffect, useRef } from 'react';
import { Platform } from 'react-native';
import * as Notifications from 'expo-notifications';
import * as Device from 'expo-device';
import { notificationsApi } from '../services/api';
import { useAuth } from '../contexts/AuthContext';

/** Configure how notifications appear when the app is in the foreground */
Notifications.setNotificationHandler({
  handleNotification: async () => ({
    shouldShowBanner: true,
    shouldShowList: true,
    shouldPlaySound: true,
    shouldSetBadge: true,
  }),
});

/**
 * Registers the device's Expo push token with the backend.
 * Only runs when the user is authenticated and on a physical device.
 * Re-registers if the token changes.
 */
export function usePushTokenRegistration() {
  const { isAuthenticated } = useAuth();
  const registeredTokenRef = useRef<string | null>(null);

  useEffect(() => {
    if (!isAuthenticated) return;

    let cancelled = false;

    async function registerToken() {
      // Push notifications only work on physical devices
      if (!Device.isDevice) return;

      const { status: existingStatus } = await Notifications.getPermissionsAsync();
      let finalStatus = existingStatus;

      if (existingStatus !== 'granted') {
        const { status } = await Notifications.requestPermissionsAsync();
        finalStatus = status;
      }

      if (finalStatus !== 'granted') return;

      const tokenData = await Notifications.getExpoPushTokenAsync();
      const token = tokenData.data;

      // Skip if already registered with this token
      if (cancelled || token === registeredTokenRef.current) return;

      const platform = Platform.OS === 'ios' ? 'ios' : 'android';
      const deviceName = Device.deviceName ?? undefined;

      await notificationsApi.registerDevice({
        expo_push_token: token,
        platform: platform as 'ios' | 'android',
        device_name: deviceName,
      });

      registeredTokenRef.current = token;
    }

    registerToken().catch(() => {
      // Silently fail - push token registration is non-critical
    });

    return () => {
      cancelled = true;
    };
  }, [isAuthenticated]);
}
