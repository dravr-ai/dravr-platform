// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Keeps the signed-in phone on the push list, opens the screen a tapped notification names, and mirrors the unread count on the app icon
// ABOUTME: Mounted once in the signed-in app group, so nothing here runs at cold start before sign-in

import { useEffect, useRef } from 'react';
import { Platform } from 'react-native';
import type { NotificationResponse } from 'expo-notifications';
import { useRouter } from 'expo-router';
import { useQueryClient } from '@tanstack/react-query';
import { QUERY_KEYS, mobileNotificationTarget } from '@pierre/shared-constants';
import { useAuth } from '../contexts/AuthContext';
import { loadNotifications } from '../services/expoNotifications';
import { canHoldPushToken, registerPushDevice } from '../services/pushNotifications';
import { useUnreadCount } from './useNotifications';

/**
 * The phone's push lifecycle for the athlete signed in on it.
 *
 * - **Registration.** On entering the app, and again for each rotation of the
 *   device's push token, the install is put on the athlete's push list
 *   ({@link registerPushDevice}), which asks for permission the first time.
 *   Signing out takes it off again, in the auth context, where the session it
 *   needs is still alive.
 * - **A tap.** A notification opens the screen its payload names, resolved by
 *   the same shared map the notification centre uses; a payload that names
 *   none just brings the app forward. A tap that launched the app is read
 *   once, at mount.
 * - **A push while the app is open.** The system shows nothing then (the
 *   app is the surface), so the unread count is refreshed instead of waiting
 *   for its next poll.
 * - **The icon badge.** Follows the unread count, so reading a notification in
 *   the app lowers the number a push set. It waits for a count that actually
 *   arrived: a failed read is not a count of zero.
 */
export function usePushNotifications(): void {
  const { user, isAuthenticated } = useAuth();
  const router = useRouter();
  const queryClient = useQueryClient();
  const { unreadCount, isSuccess: unreadCountKnown } = useUnreadCount();
  const userId = isAuthenticated ? user?.id : undefined;

  useEffect(() => {
    const notifications = loadNotifications();
    if (!userId || !notifications || !canHoldPushToken()) return undefined;
    void registerPushDevice();
    // Expo mints its token from the device's own, so a rotated device token is
    // registered again and the server holds the Expo token it now maps to.
    // Both platforms also announce here the token every fetch hands out, the
    // registration's own included, so the listener registers the token it was
    // handed: fetching another would announce it again, and register without
    // end.
    const rotation = notifications.addPushTokenListener((devicePushToken) => {
      void registerPushDevice(devicePushToken);
    });
    return () => rotation.remove();
  }, [userId]);

  // The tap that launched the app can reach both the last-response read and the
  // listener; the notification it names opens once.
  const openedNotificationId = useRef<string | null>(null);

  useEffect(() => {
    // The web build has no notification responses: expo-notifications throws
    // there for the last one.
    const notifications = loadNotifications();
    if (Platform.OS === 'web' || !notifications) return undefined;
    const open = (response: NotificationResponse): void => {
      // The last response outlives the tap that made it, and would reopen the
      // same screen the next time this layout mounts.
      notifications.clearLastNotificationResponse();
      if (response.actionIdentifier !== notifications.DEFAULT_ACTION_IDENTIFIER) return;
      const { identifier, content } = response.notification.request;
      if (openedNotificationId.current === identifier) return;
      openedNotificationId.current = identifier;
      const target = mobileNotificationTarget(content.data);
      if (target) {
        router.push(target as never);
      }
    };
    const launchedBy = notifications.getLastNotificationResponse();
    if (launchedBy) open(launchedBy);
    const taps = notifications.addNotificationResponseReceivedListener(open);
    return () => taps.remove();
  }, [router]);

  useEffect(() => {
    const notifications = loadNotifications();
    if (!notifications) return undefined;
    const arrivals = notifications.addNotificationReceivedListener(() => {
      void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.notifications.unreadCount() });
    });
    return () => arrivals.remove();
  }, [queryClient]);

  useEffect(() => {
    const notifications = loadNotifications();
    if (!unreadCountKnown || !notifications) return;
    notifications.setBadgeCountAsync(unreadCount).catch((error: unknown) => {
      console.warn('App badge update failed:', String(error));
    });
  }, [unreadCount, unreadCountKnown]);
}
