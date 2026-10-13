// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the phone's push registration: permission, the Expo token sent to the server, and sign-out deactivation
// ABOUTME: expo-notifications and the API are scripted; the stored device id and the Android channels are the real code

import AsyncStorage from '@react-native-async-storage/async-storage';
import * as Notifications from 'expo-notifications';
import { Platform } from 'react-native';
import { NOTIFICATION_CATEGORIES } from '@pierre/shared-constants';
import { notificationsApi } from '../src/services/api';
import {
  PUSH_DEVICE_STORAGE_KEY,
  PUSH_UNREGISTER_TIMEOUT_MS,
  canHoldPushToken,
  registerPushDevice,
  unregisterPushDevice,
} from '../src/services/pushNotifications';

jest.mock('../src/services/api', () => ({
  notificationsApi: {
    registerDevice: jest.fn(),
    deactivateDevice: jest.fn(),
  },
}));

// A physical phone unless a test says it is a simulator.
let mockIsDevice = true;
let mockDeviceName: string | null = 'Phil iPhone';
jest.mock('expo-device', () => ({
  get isDevice() {
    return mockIsDevice;
  },
  get deviceName() {
    return mockDeviceName;
  },
}));

// A standalone build unless a test says it runs in Expo Go.
let mockExecutionEnvironment = 'standalone';
jest.mock('expo-constants', () => ({
  __esModule: true,
  default: {
    get executionEnvironment() {
      return mockExecutionEnvironment;
    },
  },
  ExecutionEnvironment: { Bare: 'bare', Standalone: 'standalone', StoreClient: 'storeClient' },
}));

// The catalogue is not loaded here; a channel's name is the key it resolves.
jest.mock('@pierre/i18n', () => ({
  i18n: { t: (key: string) => `t:${key}` },
}));

const registerDevice = notificationsApi.registerDevice as jest.Mock;
const deactivateDevice = notificationsApi.deactivateDevice as jest.Mock;
const getPermissions = Notifications.getPermissionsAsync as jest.Mock;
const requestPermissions = Notifications.requestPermissionsAsync as jest.Mock;
const getExpoPushToken = Notifications.getExpoPushTokenAsync as jest.Mock;
const setChannel = Notifications.setNotificationChannelAsync as jest.Mock;
const setBadge = Notifications.setBadgeCountAsync as jest.Mock;

const EXPO_TOKEN = 'ExponentPushToken[abc123]';

function permission(status: 'granted' | 'denied' | 'undetermined') {
  return { status, granted: status === 'granted', canAskAgain: status !== 'denied', expires: 'never' };
}

function registeredRow(id = 'device-1') {
  return {
    id,
    user_id: 'user-1',
    tenant_id: 'tenant-1',
    expo_push_token: EXPO_TOKEN,
    platform: 'ios',
    device_name: 'Phil iPhone',
    active: true,
    created_at: '2026-10-09T12:00:00Z',
  };
}

describe('push registration', () => {
  let warnSpy: jest.SpyInstance;

  beforeEach(async () => {
    jest.clearAllMocks();
    await AsyncStorage.clear();
    mockIsDevice = true;
    mockDeviceName = 'Phil iPhone';
    mockExecutionEnvironment = 'standalone';
    getPermissions.mockResolvedValue(permission('granted'));
    getExpoPushToken.mockResolvedValue({ type: 'expo', data: EXPO_TOKEN });
    registerDevice.mockResolvedValue(registeredRow());
    deactivateDevice.mockResolvedValue(undefined);
    warnSpy = jest.spyOn(console, 'warn').mockImplementation(() => {});
  });

  afterEach(() => {
    warnSpy.mockRestore();
  });

  describe('registerPushDevice', () => {
    it('registers the Expo token with the platform and device name, and remembers the row it got', async () => {
      const device = await registerPushDevice();

      expect(registerDevice).toHaveBeenCalledTimes(1);
      expect(registerDevice).toHaveBeenCalledWith({
        expo_push_token: EXPO_TOKEN,
        platform: 'ios',
        device_name: 'Phil iPhone',
      });
      expect(device).toEqual(registeredRow());
      expect(await AsyncStorage.getItem(PUSH_DEVICE_STORAGE_KEY)).toBe('device-1');
      expect(requestPermissions).not.toHaveBeenCalled();
    });

    it('fetches the device token itself when it is handed none', async () => {
      await registerPushDevice();

      expect(getExpoPushToken).toHaveBeenCalledTimes(1);
      expect(getExpoPushToken).toHaveBeenCalledWith({});
    });

    it('registers a device token it is handed without fetching another, which would announce it again', async () => {
      const handed = { type: 'ios' as const, data: 'apns-device-token' };

      const device = await registerPushDevice(handed);

      expect(getExpoPushToken).toHaveBeenCalledTimes(1);
      expect(getExpoPushToken).toHaveBeenCalledWith({ devicePushToken: handed });
      expect(registerDevice).toHaveBeenCalledWith(expect.objectContaining({ expo_push_token: EXPO_TOKEN }));
      expect(device?.id).toBe('device-1');
    });

    it('registers once when the token its fetch announces asks for a registration while it runs', async () => {
      // What both platforms do: the token a fetch hands out is also announced
      // to every push-token listener, and the app's listener registers what it
      // hears.
      const announced: Promise<unknown>[] = [];
      getExpoPushToken.mockImplementation(async (options: { devicePushToken?: unknown }) => {
        if (!options.devicePushToken) {
          announced.push(registerPushDevice({ type: 'ios', data: 'apns-device-token' }));
        }
        return { type: 'expo', data: EXPO_TOKEN };
      });

      const device = await registerPushDevice();
      const joined = await Promise.all(announced);

      expect(getExpoPushToken).toHaveBeenCalledTimes(1);
      expect(registerDevice).toHaveBeenCalledTimes(1);
      expect(joined).toEqual([device]);
      expect(device?.id).toBe('device-1');
    });

    it('registers again once the previous registration has settled', async () => {
      await registerPushDevice();
      await registerPushDevice({ type: 'ios', data: 'apns-device-token' });

      expect(registerDevice).toHaveBeenCalledTimes(2);
      expect(getExpoPushToken).toHaveBeenLastCalledWith({
        devicePushToken: { type: 'ios', data: 'apns-device-token' },
      });
    });

    it('keeps the row it already has when the same token registers again', async () => {
      await AsyncStorage.setItem(PUSH_DEVICE_STORAGE_KEY, 'device-1');

      await registerPushDevice();

      expect(registerDevice).toHaveBeenCalledTimes(1);
      expect(deactivateDevice).not.toHaveBeenCalled();
      expect(await AsyncStorage.getItem(PUSH_DEVICE_STORAGE_KEY)).toBe('device-1');
    });

    it('retires the row of a token a rotation replaced, and remembers the new one', async () => {
      await AsyncStorage.setItem(PUSH_DEVICE_STORAGE_KEY, 'device-old');
      registerDevice.mockResolvedValue(registeredRow('device-new'));

      const device = await registerPushDevice();

      expect(device?.id).toBe('device-new');
      expect(deactivateDevice).toHaveBeenCalledTimes(1);
      expect(deactivateDevice).toHaveBeenCalledWith('device-old');
      expect(await AsyncStorage.getItem(PUSH_DEVICE_STORAGE_KEY)).toBe('device-new');
    });

    it('still reports the registration when retiring the replaced row fails', async () => {
      await AsyncStorage.setItem(PUSH_DEVICE_STORAGE_KEY, 'device-old');
      registerDevice.mockResolvedValue(registeredRow('device-new'));
      deactivateDevice.mockRejectedValue(new Error('Network Error'));

      const device = await registerPushDevice();

      expect(device?.id).toBe('device-new');
      expect(await AsyncStorage.getItem(PUSH_DEVICE_STORAGE_KEY)).toBe('device-new');
      expect(warnSpy).toHaveBeenCalledWith('Superseded push device deactivation failed:', 'Error: Network Error');
    });

    it('asks for permission when the athlete has not answered, then registers', async () => {
      getPermissions.mockResolvedValue(permission('undetermined'));
      requestPermissions.mockResolvedValue(permission('granted'));

      const device = await registerPushDevice();

      expect(requestPermissions).toHaveBeenCalledTimes(1);
      expect(registerDevice).toHaveBeenCalledTimes(1);
      expect(device?.id).toBe('device-1');
    });

    it('registers nothing when the athlete refuses the permission sheet', async () => {
      getPermissions.mockResolvedValue(permission('undetermined'));
      requestPermissions.mockResolvedValue(permission('denied'));

      const device = await registerPushDevice();

      expect(device).toBeNull();
      expect(getExpoPushToken).not.toHaveBeenCalled();
      expect(registerDevice).not.toHaveBeenCalled();
      expect(await AsyncStorage.getItem(PUSH_DEVICE_STORAGE_KEY)).toBeNull();
    });

    it('does not ask again once the athlete has refused', async () => {
      getPermissions.mockResolvedValue(permission('denied'));

      await registerPushDevice();

      expect(requestPermissions).not.toHaveBeenCalled();
      expect(registerDevice).not.toHaveBeenCalled();
    });

    it('takes a device it registered earlier off the list when the permission is later withdrawn', async () => {
      await AsyncStorage.setItem(PUSH_DEVICE_STORAGE_KEY, 'device-9');
      getPermissions.mockResolvedValue(permission('denied'));

      const device = await registerPushDevice();

      expect(device).toBeNull();
      expect(deactivateDevice).toHaveBeenCalledWith('device-9');
      expect(registerDevice).not.toHaveBeenCalled();
      expect(await AsyncStorage.getItem(PUSH_DEVICE_STORAGE_KEY)).toBeNull();
    });

    it('leaves the device name out when the phone does not report one', async () => {
      mockDeviceName = null;

      await registerPushDevice();

      expect(registerDevice).toHaveBeenCalledWith({
        expo_push_token: EXPO_TOKEN,
        platform: 'ios',
        device_name: undefined,
      });
    });

    it('registers an Android phone under the android platform', async () => {
      const platform = jest.replaceProperty(Platform, 'OS', 'android');
      try {
        await registerPushDevice();
      } finally {
        platform.restore();
      }

      expect(registerDevice).toHaveBeenCalledWith(expect.objectContaining({ platform: 'android' }));
    });

    it('does nothing on a simulator, which holds no push credentials and would put a sheet over the e2e flows', async () => {
      mockIsDevice = false;

      const device = await registerPushDevice();

      expect(device).toBeNull();
      expect(getPermissions).not.toHaveBeenCalled();
      expect(requestPermissions).not.toHaveBeenCalled();
      expect(registerDevice).not.toHaveBeenCalled();
    });

    it('does nothing in Expo Go on Android, where asking for a push token throws', async () => {
      mockExecutionEnvironment = 'storeClient';
      const platform = jest.replaceProperty(Platform, 'OS', 'android');
      let device: unknown;
      let holds: boolean | undefined;
      try {
        holds = canHoldPushToken();
        device = await registerPushDevice();
      } finally {
        platform.restore();
      }

      expect(holds).toBe(false);
      expect(device).toBeNull();
      expect(getExpoPushToken).not.toHaveBeenCalled();
      expect(registerDevice).not.toHaveBeenCalled();
    });

    it('still registers from Expo Go on iOS, which kept remote notifications', async () => {
      mockExecutionEnvironment = 'storeClient';
      const platform = jest.replaceProperty(Platform, 'OS', 'ios');
      try {
        expect(canHoldPushToken()).toBe(true);
      } finally {
        platform.restore();
      }
    });

    it('does nothing in the web build, which has no Expo push channel', async () => {
      const platform = jest.replaceProperty(Platform, 'OS', 'web');
      let device: unknown;
      try {
        device = await registerPushDevice();
      } finally {
        platform.restore();
      }

      expect(device).toBeNull();
      expect(getPermissions).not.toHaveBeenCalled();
      expect(registerDevice).not.toHaveBeenCalled();
    });

    it('resolves to null and logs only the message when the token cannot be minted', async () => {
      getExpoPushToken.mockRejectedValue(new Error('Default FirebaseApp is not initialized'));

      const device = await registerPushDevice();

      expect(device).toBeNull();
      expect(registerDevice).not.toHaveBeenCalled();
      expect(warnSpy).toHaveBeenCalledWith('Push registration failed:', 'Error: Default FirebaseApp is not initialized');
    });

    it('resolves to null when the server refuses the registration, without logging the token', async () => {
      registerDevice.mockRejectedValue(new Error('Request failed with status code 400'));

      const device = await registerPushDevice();

      expect(device).toBeNull();
      expect(await AsyncStorage.getItem(PUSH_DEVICE_STORAGE_KEY)).toBeNull();
      expect(JSON.stringify(warnSpy.mock.calls)).not.toContain(EXPO_TOKEN);
    });

    it('creates one Android channel per notification category, on the id the server sends each push to', async () => {
      const platform = jest.replaceProperty(Platform, 'OS', 'android');
      try {
        await registerPushDevice();
      } finally {
        platform.restore();
      }

      const channelIds = setChannel.mock.calls.map(([id]: [string]) => id);
      expect(channelIds.sort()).toEqual([...NOTIFICATION_CATEGORIES].sort());
      const options = Object.fromEntries(
        setChannel.mock.calls.map(([id, channel]: [string, { name: string; importance: number }]) => [id, channel]),
      );
      expect(options.coach).toEqual({
        name: 't:notifPrefs.catAgent',
        importance: Notifications.AndroidImportance.MAX,
      });
      expect(options.system).toEqual({
        name: 't:notifPrefs.catSystem',
        importance: Notifications.AndroidImportance.LOW,
      });
      expect(options.training.importance).toBe(Notifications.AndroidImportance.HIGH);
    });

    it('creates no Android channels on iOS', async () => {
      await registerPushDevice();

      expect(setChannel).not.toHaveBeenCalled();
    });
  });

  describe('unregisterPushDevice', () => {
    it('deactivates the device row this install registered, forgets it, and clears the icon badge', async () => {
      await AsyncStorage.setItem(PUSH_DEVICE_STORAGE_KEY, 'device-1');

      await unregisterPushDevice();

      expect(deactivateDevice).toHaveBeenCalledTimes(1);
      expect(deactivateDevice).toHaveBeenCalledWith('device-1');
      expect(await AsyncStorage.getItem(PUSH_DEVICE_STORAGE_KEY)).toBeNull();
      expect(setBadge).toHaveBeenCalledWith(0);
    });

    it('asks the server for nothing when this install never registered', async () => {
      await unregisterPushDevice();

      expect(deactivateDevice).not.toHaveBeenCalled();
      expect(setBadge).toHaveBeenCalledWith(0);
    });

    it('still finishes, and forgets the row, when the server cannot be reached', async () => {
      await AsyncStorage.setItem(PUSH_DEVICE_STORAGE_KEY, 'device-1');
      deactivateDevice.mockRejectedValue(new Error('Network Error'));

      await expect(unregisterPushDevice()).resolves.toBeUndefined();

      expect(await AsyncStorage.getItem(PUSH_DEVICE_STORAGE_KEY)).toBeNull();
      expect(warnSpy).toHaveBeenCalledWith('Push device deactivation failed:', 'Error: Network Error');
    });

    it('does not hold the sign-out on a badge reset that never settles', async () => {
      setBadge.mockReturnValueOnce(new Promise(() => {}));

      await expect(unregisterPushDevice()).resolves.toBeUndefined();

      expect(setBadge).toHaveBeenCalledWith(0);
    });

    it('logs a badge reset the phone refuses without failing the sign-out', async () => {
      setBadge.mockRejectedValueOnce(new Error('Badge unavailable'));

      await expect(unregisterPushDevice()).resolves.toBeUndefined();

      expect(warnSpy).toHaveBeenCalledWith('Push badge reset failed:', 'Error: Badge unavailable');
    });

    it('gives up on a server that never answers instead of holding the sign-out', async () => {
      jest.useFakeTimers();
      try {
        await AsyncStorage.setItem(PUSH_DEVICE_STORAGE_KEY, 'device-1');
        deactivateDevice.mockReturnValue(new Promise(() => {}));

        let finished = false;
        const done = unregisterPushDevice().then(() => {
          finished = true;
        });

        await jest.advanceTimersByTimeAsync(PUSH_UNREGISTER_TIMEOUT_MS - 1);
        expect(finished).toBe(false);

        await jest.advanceTimersByTimeAsync(1);
        await done;
        expect(finished).toBe(true);
        expect(await AsyncStorage.getItem(PUSH_DEVICE_STORAGE_KEY)).toBeNull();
        expect(setBadge).toHaveBeenCalledWith(0);
      } finally {
        jest.useRealTimers();
      }
    });
  });
});
