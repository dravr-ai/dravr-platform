// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the push lifecycle of the signed-in app: registration, token rotation, a tap's destination, and the icon badge
// ABOUTME: expo-notifications, the router and the unread count are scripted; the hook's own wiring and the shared tap resolver are real

import React, { type ReactNode } from 'react';
import { renderHook, waitFor } from '@testing-library/react-native';
import { Platform } from 'react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import * as Notifications from 'expo-notifications';
import { QUERY_KEYS } from '@pierre/shared-constants';
import { registerPushDevice } from '../src/services/pushNotifications';
import { usePushNotifications } from '../src/hooks/usePushNotifications';

const mockPush = jest.fn();
jest.mock('expo-router', () => ({
  useRouter: () => ({ push: mockPush }),
}));

let mockAuth: { isAuthenticated: boolean; user: { id: string } | null };
jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => mockAuth,
}));

let mockCanHoldPushToken = true;
jest.mock('../src/services/pushNotifications', () => ({
  canHoldPushToken: () => mockCanHoldPushToken,
  registerPushDevice: jest.fn(() => Promise.resolve(null)),
}));

let mockUnread: { unreadCount: number; isSuccess: boolean };
jest.mock('../src/hooks/useNotifications', () => ({
  useUnreadCount: () => mockUnread,
}));

const addPushTokenListener = Notifications.addPushTokenListener as jest.Mock;
const addReceivedListener = Notifications.addNotificationReceivedListener as jest.Mock;
const addResponseListener = Notifications.addNotificationResponseReceivedListener as jest.Mock;
const getLastResponse = Notifications.getLastNotificationResponse as jest.Mock;
const clearLastResponse = Notifications.clearLastNotificationResponse as jest.Mock;
const setBadge = Notifications.setBadgeCountAsync as jest.Mock;
const register = registerPushDevice as jest.Mock;

const DEFAULT_ACTION = 'expo.modules.notifications.actions.DEFAULT';

/** A tap on the notification `identifier`, whose push carried `data`. */
function tap(data: Record<string, unknown>, actionIdentifier = DEFAULT_ACTION, identifier = 'push-1') {
  return { actionIdentifier, notification: { request: { identifier, content: { data } } } };
}

function setup() {
  const client = new QueryClient();
  const invalidate = jest.spyOn(client, 'invalidateQueries');
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  const view = renderHook(() => usePushNotifications(), { wrapper });
  return { ...view, invalidate };
}

/** The listener a subscription call was handed, and the `remove` it got back. */
function subscribed(mock: jest.Mock) {
  const index = mock.mock.calls.length - 1;
  return {
    listener: mock.mock.calls[index][0] as (event: unknown) => void,
    remove: mock.mock.results[index].value.remove as jest.Mock,
  };
}

describe('usePushNotifications', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockAuth = { isAuthenticated: true, user: { id: 'user-1' } };
    mockUnread = { unreadCount: 0, isSuccess: false };
    mockCanHoldPushToken = true;
    getLastResponse.mockReturnValue(null);
  });

  describe('registration', () => {
    it('puts the phone on the push list when the signed-in app opens', async () => {
      setup();

      await waitFor(() => expect(register).toHaveBeenCalledTimes(1));
      expect(addPushTokenListener).toHaveBeenCalledTimes(1);
    });

    it('registers again when the device push token rotates, with the token it was handed', async () => {
      setup();
      await waitFor(() => expect(register).toHaveBeenCalledTimes(1));
      expect(register).toHaveBeenCalledWith();

      subscribed(addPushTokenListener).listener({ type: 'ios', data: 'rotated-device-token' });

      expect(register).toHaveBeenCalledTimes(2);
      // Registering the announced token, rather than fetching one, is what
      // keeps the fetch's own announcement from starting another registration.
      expect(register).toHaveBeenLastCalledWith({ type: 'ios', data: 'rotated-device-token' });
    });

    it('registers nothing, and listens for no rotation, before anyone is signed in', () => {
      mockAuth = { isAuthenticated: false, user: null };

      setup();

      expect(register).not.toHaveBeenCalled();
      expect(addPushTokenListener).not.toHaveBeenCalled();
    });

    it('registers nothing, and listens for no rotation, where the install cannot hold a push token', () => {
      // An emulator, or Expo Go on Android, where asking for a token or
      // listening for one throws.
      mockCanHoldPushToken = false;

      setup();

      expect(register).not.toHaveBeenCalled();
      expect(addPushTokenListener).not.toHaveBeenCalled();
    });

    it('registers nothing for an account that is not yet active', () => {
      mockAuth = { isAuthenticated: false, user: { id: 'user-1' } };

      setup();

      expect(register).not.toHaveBeenCalled();
    });

    it('stops listening for the rotation when the app group unmounts', async () => {
      const { unmount } = setup();
      await waitFor(() => expect(register).toHaveBeenCalledTimes(1));
      const { remove } = subscribed(addPushTokenListener);

      unmount();

      expect(remove).toHaveBeenCalledTimes(1);
    });

    it('registers the next athlete when the signed-in user changes', async () => {
      const { rerender } = setup();
      await waitFor(() => expect(register).toHaveBeenCalledTimes(1));

      mockAuth = { isAuthenticated: true, user: { id: 'user-2' } };
      rerender({});

      await waitFor(() => expect(register).toHaveBeenCalledTimes(2));
    });
  });

  describe('a tapped notification', () => {
    it('opens the thread an agent message names', () => {
      setup();

      subscribed(addResponseListener).listener(tap({ screen: 'coach', action: 'chat', id: 'conv-7' }));

      expect(mockPush).toHaveBeenCalledTimes(1);
      expect(mockPush).toHaveBeenCalledWith({
        pathname: '/(app)/chat/[conversationId]',
        params: { conversationId: 'conv-7' },
      });
    });

    it('opens the app and nothing more when the payload names no destination', () => {
      setup();

      subscribed(addResponseListener).listener(tap({ params: { score: '32' } }));

      expect(mockPush).not.toHaveBeenCalled();
    });

    it('ignores a tap on an action button, which is not the notification itself', () => {
      setup();

      subscribed(addResponseListener).listener(
        tap({ screen: 'coach', action: 'chat', id: 'conv-7' }, 'reply'),
      );

      expect(mockPush).not.toHaveBeenCalled();
    });

    it('forgets the response once handled, so the next mount does not reopen it', () => {
      setup();

      subscribed(addResponseListener).listener(tap({ screen: 'coach', action: 'chat', id: 'conv-7' }));

      expect(clearLastResponse).toHaveBeenCalledTimes(1);
    });

    it('opens the destination of the tap that launched the app', () => {
      getLastResponse.mockReturnValue(tap({ screen: 'coach', action: 'chat', id: 'conv-3' }));

      setup();

      expect(mockPush).toHaveBeenCalledTimes(1);
      expect(mockPush).toHaveBeenCalledWith({
        pathname: '/(app)/chat/[conversationId]',
        params: { conversationId: 'conv-3' },
      });
      expect(clearLastResponse).toHaveBeenCalledTimes(1);
    });

    it('opens the launching tap once when the listener delivers it as well', () => {
      const launch = tap({ screen: 'coach', action: 'chat', id: 'conv-3' }, DEFAULT_ACTION, 'push-launch');
      getLastResponse.mockReturnValue(launch);

      setup();
      subscribed(addResponseListener).listener(launch);

      expect(mockPush).toHaveBeenCalledTimes(1);
      expect(mockPush).toHaveBeenCalledWith({
        pathname: '/(app)/chat/[conversationId]',
        params: { conversationId: 'conv-3' },
      });
    });

    it('opens each of two different notifications tapped in turn', () => {
      setup();
      const { listener } = subscribed(addResponseListener);

      listener(tap({ screen: 'coach', action: 'chat', id: 'conv-1' }, DEFAULT_ACTION, 'push-1'));
      listener(tap({ screen: 'coach', action: 'chat', id: 'conv-2' }, DEFAULT_ACTION, 'push-2'));

      expect(mockPush).toHaveBeenCalledTimes(2);
      expect(mockPush).toHaveBeenLastCalledWith({
        pathname: '/(app)/chat/[conversationId]',
        params: { conversationId: 'conv-2' },
      });
    });

    it('opens nothing when the app was not launched by a notification', () => {
      setup();

      expect(getLastResponse).toHaveBeenCalledTimes(1);
      expect(mockPush).not.toHaveBeenCalled();
      expect(clearLastResponse).not.toHaveBeenCalled();
    });

    it('reads no notification response in the web build, which has none', () => {
      const platform = jest.replaceProperty(Platform, 'OS', 'web');
      try {
        setup();
      } finally {
        platform.restore();
      }

      expect(getLastResponse).not.toHaveBeenCalled();
      expect(addResponseListener).not.toHaveBeenCalled();
      expect(mockPush).not.toHaveBeenCalled();
    });

    it('stops listening for taps when the app group unmounts', () => {
      const { unmount } = setup();
      const { remove } = subscribed(addResponseListener);

      unmount();

      expect(remove).toHaveBeenCalledTimes(1);
    });
  });

  describe('a push that arrives while the app is open', () => {
    it('refreshes the unread count instead of waiting for its next poll', () => {
      const { invalidate } = setup();

      subscribed(addReceivedListener).listener({ request: { content: { data: {} } } });

      expect(invalidate).toHaveBeenCalledWith({ queryKey: QUERY_KEYS.notifications.unreadCount() });
    });
  });

  describe('the icon badge', () => {
    it('shows the unread count once it has arrived', async () => {
      mockUnread = { unreadCount: 3, isSuccess: true };

      setup();

      await waitFor(() => expect(setBadge).toHaveBeenCalledWith(3));
    });

    it('lowers the badge when notifications are read in the app', async () => {
      mockUnread = { unreadCount: 3, isSuccess: true };
      const { rerender } = setup();
      await waitFor(() => expect(setBadge).toHaveBeenCalledWith(3));

      mockUnread = { unreadCount: 1, isSuccess: true };
      rerender({});

      await waitFor(() => expect(setBadge).toHaveBeenLastCalledWith(1));
    });

    it('leaves the badge alone while the count has not arrived, rather than reading it as zero', () => {
      mockUnread = { unreadCount: 0, isSuccess: false };

      setup();

      expect(setBadge).not.toHaveBeenCalled();
    });

    it('survives a phone that refuses the badge', async () => {
      const warn = jest.spyOn(console, 'warn').mockImplementation(() => {});
      setBadge.mockRejectedValueOnce(new Error('badge unavailable'));
      mockUnread = { unreadCount: 2, isSuccess: true };

      setup();

      await waitFor(() => expect(warn).toHaveBeenCalledWith('App badge update failed:', 'Error: badge unavailable'));
      warn.mockRestore();
    });
  });
});
