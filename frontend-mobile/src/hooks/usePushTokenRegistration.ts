// ABOUTME: Registers Expo push token with the backend on app start
// ABOUTME: Handles permission requests, token retrieval, badge sync, and Android channel setup

import { useEffect, useRef } from 'react';
import { Platform } from 'react-native';
import * as Notifications from 'expo-notifications';
import * as Device from 'expo-device';
import { notificationsApi } from '../services/api';
import { useAuth } from '../contexts/AuthContext';

/** Notification channel definitions matching backend categories */
const NOTIFICATION_CHANNELS: { id: string; name: string; importance: number }[] = [
  { id: 'training', name: 'Training & Activity', importance: Notifications.AndroidImportance.HIGH },
  { id: 'recovery', name: 'Recovery & Health', importance: Notifications.AndroidImportance.HIGH },
  { id: 'social', name: 'Social', importance: Notifications.AndroidImportance.DEFAULT },
  { id: 'coach', name: 'Coach Messages', importance: Notifications.AndroidImportance.MAX },
  { id: 'achievement', name: 'Achievements', importance: Notifications.AndroidImportance.DEFAULT },
  { id: 'system', name: 'System', importance: Notifications.AndroidImportance.LOW },
  { id: 'ai', name: 'AI Insights', importance: Notifications.AndroidImportance.DEFAULT },
  { id: 'reminders', name: 'Reminders', importance: Notifications.AndroidImportance.HIGH },
];

/** Configure how notifications appear when the app is in the foreground */
Notifications.setNotificationHandler({
  handleNotification: async () => ({
    shouldShowBanner: true,
    shouldShowList: true,
    shouldPlaySound: true,
    shouldSetBadge: true,
  }),
});

/** Register Android notification channels matching backend categories */
async function registerAndroidChannels() {
  if (Platform.OS !== 'android') return;

  for (const channel of NOTIFICATION_CHANNELS) {
    await Notifications.setNotificationChannelAsync(channel.id, {
      name: channel.name,
      importance: channel.importance,
      sound: 'default',
      vibrationPattern: [0, 250, 250, 250],
      enableLights: true,
    });
  }
}

/** Sync badge count with the server after receiving a notification */
async function syncBadgeCount() {
  try {
    const response = await notificationsApi.badgeSync();
    await Notifications.setBadgeCountAsync(response.count);
  } catch {
    // Badge sync is non-critical — silently ignore errors
  }
}

/**
 * Registers the device's Expo push token with the backend.
 * Only runs when the user is authenticated and on a physical device.
 * Re-registers if the token changes. Sets up Android notification channels
 * and syncs badge counts on incoming notifications.
 */
export function usePushTokenRegistration() {
  const { isAuthenticated } = useAuth();
  const registeredTokenRef = useRef<string | null>(null);

  // Register Android channels on mount
  useEffect(() => {
    registerAndroidChannels().catch(() => {
      // Channel registration is non-critical
    });
  }, []);

  // Listen for incoming notifications and sync badge count
  useEffect(() => {
    if (!isAuthenticated) return;

    const subscription = Notifications.addNotificationReceivedListener(() => {
      syncBadgeCount();
    });

    return () => subscription.remove();
  }, [isAuthenticated]);

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

      // Sync badge on registration
      await syncBadgeCount();
    }

    registerToken().catch(() => {
      // Silently fail - push token registration is non-critical
    });

    return () => {
      cancelled = true;
    };
  }, [isAuthenticated]);
}
