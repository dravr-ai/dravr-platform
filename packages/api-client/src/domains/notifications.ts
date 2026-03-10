// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Notifications domain API - device tokens, preferences, feed
// ABOUTME: Handles push notification management including device registration and notification feed

import type { AxiosInstance } from 'axios';
import type {
  DeviceToken,
  RegisterDeviceTokenRequest,
  NotificationPreferencesResponse,
  UpdateNotificationPreferenceRequest,
  NotificationPreferenceItem,
  NotificationFeedResponse,
  UnreadCountResponse,
  MarkAllReadResponse,
  ListNotificationsParams,
} from '@pierre/shared-types';
import { ENDPOINTS } from '../core/endpoints';

// Re-export types for consumers
export type {
  DeviceToken,
  RegisterDeviceTokenRequest,
  NotificationPreferencesResponse,
  UpdateNotificationPreferenceRequest,
  NotificationPreferenceItem,
  NotificationFeedResponse,
  UnreadCountResponse,
  MarkAllReadResponse,
  ListNotificationsParams,
};

/**
 * Creates the notifications API methods bound to an axios instance.
 */
export function createNotificationsApi(axios: AxiosInstance) {
  return {
    // ==================== DEVICE TOKENS ====================

    /**
     * Register a device token for push notifications.
     */
    async registerDevice(request: RegisterDeviceTokenRequest): Promise<DeviceToken> {
      const response = await axios.post<DeviceToken>(ENDPOINTS.NOTIFICATIONS.DEVICES, request);
      return response.data;
    },

    /**
     * List active device tokens for the current user.
     */
    async listDevices(): Promise<DeviceToken[]> {
      const response = await axios.get<DeviceToken[]>(ENDPOINTS.NOTIFICATIONS.DEVICES);
      return response.data;
    },

    /**
     * Deactivate a device token.
     */
    async deactivateDevice(deviceId: string): Promise<void> {
      await axios.delete(ENDPOINTS.NOTIFICATIONS.DEVICE(deviceId));
    },

    // ==================== PREFERENCES ====================

    /**
     * Get notification preferences for all categories.
     */
    async getPreferences(): Promise<NotificationPreferencesResponse> {
      const response = await axios.get<NotificationPreferencesResponse>(
        ENDPOINTS.NOTIFICATIONS.PREFERENCES,
      );
      return response.data;
    },

    /**
     * Update notification preferences for a category.
     */
    async updatePreference(
      request: UpdateNotificationPreferenceRequest,
    ): Promise<NotificationPreferenceItem> {
      const response = await axios.put<NotificationPreferenceItem>(
        ENDPOINTS.NOTIFICATIONS.PREFERENCES,
        request,
      );
      return response.data;
    },

    // ==================== NOTIFICATION FEED ====================

    /**
     * List notifications with optional filtering and pagination.
     */
    async listNotifications(params?: ListNotificationsParams): Promise<NotificationFeedResponse> {
      const urlParams = new URLSearchParams();
      if (params?.limit) urlParams.append('limit', params.limit.toString());
      if (params?.offset) urlParams.append('offset', params.offset.toString());
      if (params?.category) urlParams.append('category', params.category);
      if (params?.unread_only) urlParams.append('unread_only', 'true');

      const queryString = urlParams.toString();
      const url = queryString
        ? `${ENDPOINTS.NOTIFICATIONS.FEED}?${queryString}`
        : ENDPOINTS.NOTIFICATIONS.FEED;

      const response = await axios.get<NotificationFeedResponse>(url);
      return response.data;
    },

    /**
     * Get the count of unread notifications.
     */
    async getUnreadCount(): Promise<UnreadCountResponse> {
      const response = await axios.get<UnreadCountResponse>(ENDPOINTS.NOTIFICATIONS.UNREAD_COUNT);
      return response.data;
    },

    /**
     * Mark a single notification as read.
     */
    async markAsRead(notificationId: string): Promise<void> {
      await axios.post(ENDPOINTS.NOTIFICATIONS.READ(notificationId));
    },

    /**
     * Mark all notifications as read.
     */
    async markAllAsRead(): Promise<MarkAllReadResponse> {
      const response = await axios.post<MarkAllReadResponse>(ENDPOINTS.NOTIFICATIONS.READ_ALL);
      return response.data;
    },

    /**
     * Delete a notification.
     */
    async deleteNotification(notificationId: string): Promise<void> {
      await axios.delete(ENDPOINTS.NOTIFICATIONS.DELETE(notificationId));
    },
  };
}

export type NotificationsApi = ReturnType<typeof createNotificationsApi>;
