// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The phone's side of push delivery: notification permission, the Expo push token, and its registration with the server
// ABOUTME: Remembers the device row this install registered so sign-out deactivates exactly that one

import AsyncStorage from '@react-native-async-storage/async-storage';
import * as Device from 'expo-device';
import type { AndroidImportance, DevicePushToken } from 'expo-notifications';
import { Platform } from 'react-native';
import { i18n } from '@pierre/i18n';
import { NOTIFICATION_CATEGORIES, NOTIFICATION_CATEGORY_META } from '@pierre/shared-constants';
import type { DevicePlatform, DeviceToken, NotificationCategory } from '@pierre/shared-types';
import { notificationsApi } from './api';
import { clientHasRemotePush, loadNotifications, type ExpoNotifications } from './expoNotifications';

/** Where the id of the device row this install registered is kept. */
export const PUSH_DEVICE_STORAGE_KEY = 'dravr.push_device_id';

/**
 * How long sign-out waits for the server to take this device off the push
 * list. The shared client's request timeout is five minutes (slow LLM turns),
 * far too long to hold a logout on a dead connection.
 */
export const PUSH_UNREGISTER_TIMEOUT_MS = 5_000;

/**
 * The importance of each category's Android channel. The server sends every
 * push on the channel named by its category, so these ids are the
 * categories themselves; the athlete tunes each one in the system settings.
 */
const CHANNEL_IMPORTANCE: Record<NotificationCategory, keyof typeof AndroidImportance> = {
  training: 'HIGH',
  recovery: 'HIGH',
  coach: 'MAX',
  achievement: 'DEFAULT',
  system: 'LOW',
  ai: 'DEFAULT',
  reminders: 'HIGH',
};

/**
 * The platform the server files this device under, or null where the phone
 * cannot hold a push token: a simulator or emulator carries no push
 * credentials (and a permission sheet over the end-to-end flows), Expo Go on
 * Android has had no remote notifications since SDK 53 (asking it for a token,
 * or listening for one, throws), and the web build has no Expo push channel.
 */
function pushPlatform(): DevicePlatform | null {
  if (!Device.isDevice || !clientHasRemotePush()) return null;
  return Platform.OS === 'ios' || Platform.OS === 'android' ? Platform.OS : null;
}

/** Whether this install can hold a push token, and so join the push list at all. */
export function canHoldPushToken(): boolean {
  return pushPlatform() !== null;
}

/**
 * One Android channel per notification category, named in the athlete's
 * language from the catalogue the preferences screen already reads. Android
 * 13 shows no permission sheet until a channel exists.
 */
async function ensureAndroidChannels(notifications: ExpoNotifications): Promise<void> {
  if (Platform.OS !== 'android') return;
  await Promise.all(
    NOTIFICATION_CATEGORIES.map((category) =>
      notifications.setNotificationChannelAsync(category, {
        name: i18n.t(NOTIFICATION_CATEGORY_META[category].labelKey),
        importance: notifications.AndroidImportance[CHANNEL_IMPORTANCE[category]],
      }),
    ),
  );
}

/**
 * Whether notifications are allowed, asking once.
 *
 * The system sheet appears only while the answer is undetermined: a refusal is
 * the athlete's to reverse in the system settings, so a denied phone is not
 * asked again at every launch.
 */
async function notificationsAllowed(notifications: ExpoNotifications): Promise<boolean> {
  let permissions = await notifications.getPermissionsAsync();
  if (permissions.status === notifications.PermissionStatus.UNDETERMINED) {
    permissions = await notifications.requestPermissionsAsync();
  }
  return permissions.granted;
}

/**
 * Deactivate the row of a token this install no longer holds. A rotation mints
 * a new token and so a new row; the old one would go on being addressed.
 * Best effort: the registration it follows has already succeeded.
 */
async function retireSupersededDevice(deviceId: string): Promise<void> {
  try {
    await notificationsApi.deactivateDevice(deviceId);
  } catch (error) {
    console.warn('Superseded push device deactivation failed:', String(error));
  }
}

/**
 * The registration under way, which a second request joins instead of
 * repeating it: fetching the device token announces that token to every
 * push-token listener, so the app's own rotation listener asks for a
 * registration of the same token while this one is still running.
 */
let registrationInFlight: Promise<DeviceToken | null> | null = null;

/**
 * Put this install on the signed-in athlete's push list.
 *
 * Asks for notification permission on first use, mints the Expo push token and
 * upserts it, so running it again on every launch or token rotation is how a
 * row the server deactivated comes back. A rotation that yields a new row
 * retires the one it replaces. A phone that cannot hold a token, a
 * refused permission and a failed call all resolve to null: push is an
 * addition to the in-app feed, never a reason to interrupt the app. A refusal
 * also takes a device this install registered earlier off the list.
 *
 * `devicePushToken` is the token a push-token listener was handed. It is
 * registered as it is, because fetching one would announce it to that listener
 * again, which would register it again, without end. Without one, the token is
 * fetched.
 */
export function registerPushDevice(
  devicePushToken?: DevicePushToken,
): Promise<DeviceToken | null> {
  if (!registrationInFlight) {
    registrationInFlight = registerThisInstall(devicePushToken).finally(() => {
      registrationInFlight = null;
    });
  }
  return registrationInFlight;
}

async function registerThisInstall(
  devicePushToken: DevicePushToken | undefined,
): Promise<DeviceToken | null> {
  const platform = pushPlatform();
  const notifications = platform ? loadNotifications() : null;
  if (!platform || !notifications) return null;
  try {
    await ensureAndroidChannels(notifications);
    if (!(await notificationsAllowed(notifications))) {
      await unregisterPushDevice();
      return null;
    }
    // LIMITATION(registre#301): `registerPushDevice` gets no Expo push token on
    // Android, whose build carries no Firebase config (`android.googleServicesFile`
    // in app.config.js) for FCM to issue the device token Expo's is minted from;
    // iOS needs the APNs key EAS holds for the project. Where the request below
    // throws, the catch logs it and the in-app feed is what reaches the athlete.
    const { data: expoPushToken } = await notifications.getExpoPushTokenAsync(
      devicePushToken ? { devicePushToken } : {},
    );
    const previousDeviceId = await AsyncStorage.getItem(PUSH_DEVICE_STORAGE_KEY);
    const device = await notificationsApi.registerDevice({
      expo_push_token: expoPushToken,
      platform,
      device_name: Device.deviceName ?? undefined,
    });
    await AsyncStorage.setItem(PUSH_DEVICE_STORAGE_KEY, device.id);
    if (previousDeviceId && previousDeviceId !== device.id) {
      await retireSupersededDevice(previousDeviceId);
    }
    return device;
  } catch (error) {
    // The error's own text, never the error object: an axios error carries the
    // request that held the token.
    console.warn('Push registration failed:', String(error));
    return null;
  }
}

/** What the deactivation race settles to when the server never answered. */
const NO_ANSWER = Symbol();

/** Deactivate `deviceId`, giving up on a server that has not answered within `ms`. */
async function deactivateWithin(deviceId: string, ms: number): Promise<void> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const elapsed = new Promise<typeof NO_ANSWER>((resolve) => {
    timer = setTimeout(() => resolve(NO_ANSWER), ms);
  });
  try {
    const outcome = await Promise.race([notificationsApi.deactivateDevice(deviceId), elapsed]);
    if (outcome === NO_ANSWER) {
      console.warn(`Push device deactivation got no answer within ${ms} ms`);
    }
  } finally {
    clearTimeout(timer);
  }
}

/**
 * Take this install off the push list and clear what its pushes left on the
 * phone.
 *
 * Sign-out calls it before the session ends, because deactivating a device
 * row needs the session it belongs to; once the server has answered, the
 * signed-out phone receives nothing meant for the athlete who used it. It
 * never throws and waits at most {@link PUSH_UNREGISTER_TIMEOUT_MS}: a server
 * that cannot be reached must not keep an athlete signed in. The phone then
 * releases its push token itself ({@link releasePushDevice}), so a sign-out
 * the server never heard still stops the pushes.
 */
export async function unregisterPushDevice(): Promise<void> {
  try {
    const deviceId = await AsyncStorage.getItem(PUSH_DEVICE_STORAGE_KEY);
    if (deviceId) {
      await deactivateWithin(deviceId, PUSH_UNREGISTER_TIMEOUT_MS);
    }
  } catch (error) {
    console.warn('Push device deactivation failed:', String(error));
  }
  await releasePushDevice();
}

/**
 * Stop this install receiving pushes without asking the server.
 *
 * The session may already be gone (the refresh token died) or the phone
 * offline, so nothing here needs either. Unregistering for remote
 * notifications voids the install's push token, so the phone shows nothing
 * pushed to it from then on. Apple or Google then report the token
 * unregistered, and the server retires the row when it reads that from the
 * next push's Expo receipt (or ticket). The next sign-in asks for a token
 * again, which registers anew. Neither native call holds up a sign-out,
 * because either can go unanswered (Expo Go on the iOS simulator), and
 * neither failure fails it.
 */
export async function releasePushDevice(): Promise<void> {
  try {
    await AsyncStorage.removeItem(PUSH_DEVICE_STORAGE_KEY);
  } catch (error) {
    console.warn('Push device cleanup failed:', String(error));
  }
  const notifications = loadNotifications();
  if (canHoldPushToken()) {
    void notifications
      ?.unregisterForNotificationsAsync()
      .catch((error: unknown) => console.warn('Push token release failed:', String(error)));
  }
  void notifications
    ?.setBadgeCountAsync(0)
    .catch((error: unknown) => console.warn('Push badge reset failed:', String(error)));
}
