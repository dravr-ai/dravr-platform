// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Loads expo-notifications only in a client that has remote notifications, so Expo Go on Android never evaluates it
// ABOUTME: Importing the package subscribes to push tokens as it loads, which throws in Expo Go on Android since SDK 53

import Constants, { ExecutionEnvironment } from 'expo-constants';
import { Platform } from 'react-native';

/** The expo-notifications module. */
export type ExpoNotifications = typeof import('expo-notifications');

/**
 * Whether this client has remote notifications at all. Expo Go on Android
 * dropped them in SDK 53, and there the package throws while it loads: its
 * auto-registration subscribes to push tokens at import time. A development or
 * store build has them on both platforms.
 */
export function clientHasRemotePush(): boolean {
  return !(
    Platform.OS === 'android' && Constants.executionEnvironment === ExecutionEnvironment.StoreClient
  );
}

/**
 * expo-notifications, loaded where loading it cannot throw, or `null` in Expo
 * Go on Android, where the app runs without push. Every caller goes through
 * this rather than importing the package, because a single static import
 * would evaluate it with the app shell and take the shell down with it.
 */
export function loadNotifications(): ExpoNotifications | null {
  return clientHasRemotePush() ? (require('expo-notifications') as ExpoNotifications) : null;
}
