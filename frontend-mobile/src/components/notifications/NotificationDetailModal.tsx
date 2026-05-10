// ABOUTME: Modal overlay that displays full notification content when a notification is tapped
// ABOUTME: Shows title, body, image, category badge, timestamp, and action buttons

import React from 'react';
import {
  Modal,
  View,
  Text,
  TouchableOpacity,
  Image,
  ScrollView,
  type ViewStyle,
} from 'react-native';
import { X, ExternalLink } from 'lucide-react-native';
import { useThemeColors } from '../../constants/theme';
import {
  NOTIFICATION_CATEGORY_META,
  formatNotificationTime,
} from '../../../../packages/shared-constants/src/notifications';
import type { NotificationItem, NotificationAction } from '@pierre/shared-types';

interface NotificationDetailModalProps {
  visible: boolean;
  notification: NotificationItem | null;
  onClose: () => void;
  onAction: (item: NotificationItem, actionId: string) => void;
  onNavigate: (item: NotificationItem) => void;
}

export function NotificationDetailModal({
  visible,
  notification,
  onClose,
  onAction,
  onNavigate,
}: NotificationDetailModalProps) {
  const colors = useThemeColors();
  const modalShadow: ViewStyle = {
    shadowColor: colors.text.primary,
    shadowOffset: { width: 0, height: 8 },
    shadowOpacity: 0.4,
    shadowRadius: 16,
    elevation: 12,
  };

  if (!notification) return null;

  const meta = NOTIFICATION_CATEGORY_META[notification.category];
  const route = notification.data?.route;
  const hasRoute = typeof route === 'string' && route.length > 0;

  return (
    <Modal
      visible={visible}
      animationType="fade"
      transparent
      onRequestClose={onClose}
    >
      <TouchableOpacity
        activeOpacity={1}
        onPress={onClose}
        className="flex-1 bg-black/60 justify-center items-center px-6"
      >
        <View
          className="w-full max-w-[380px] rounded-2xl overflow-hidden"
          style={[{ backgroundColor: colors.background.elevated }, modalShadow]}
          onStartShouldSetResponder={() => true}
        >
          {/* Header with category badge and close button */}
          <View className="flex-row items-center justify-between px-5 pt-5 pb-3">
            <View className="flex-row items-center">
              <View
                className="w-2.5 h-2.5 rounded-full mr-2"
                style={{ backgroundColor: meta.color }}
              />
              <Text className="text-xs font-semibold" style={{ color: meta.color }}>
                {meta.label.toUpperCase()}
              </Text>
            </View>
            <TouchableOpacity
              onPress={onClose}
              className="w-8 h-8 items-center justify-center rounded-full bg-surface-container-low"
              hitSlop={{ top: 8, bottom: 8, left: 8, right: 8 }}
              testID="notification-detail-close"
            >
              <X size={16} color={colors.text.tertiary} />
            </TouchableOpacity>
          </View>

          <ScrollView className="max-h-[400px]" bounces={false}>
            {/* Image */}
            {notification.image_url ? (
              <Image
                source={{ uri: notification.image_url }}
                className="w-full h-48 mx-0"
                resizeMode="cover"
              />
            ) : null}

            {/* Content */}
            <View className="px-5 pb-5">
              <Text className="text-lg font-bold text-on-surface mt-3 mb-1">
                {notification.title}
              </Text>

              <Text className="text-xs text-outline mb-3">
                {formatNotificationTime(notification.created_at)}
              </Text>

              <Text className="text-sm text-on-surface leading-5">
                {notification.body}
              </Text>

              {/* Action buttons */}
              {notification.actions && notification.actions.length > 0 ? (
                <View className="flex-row mt-4 gap-2 flex-wrap">
                  {notification.actions.map((action: NotificationAction) => (
                    <TouchableOpacity
                      key={action.id}
                      className="px-4 py-2 rounded-lg"
                      style={{ backgroundColor: `${colors.pierre.violet}1F` }}
                      onPress={() => {
                        onAction(notification, action.id);
                        onClose();
                      }}
                      testID={`detail-action-${action.id}`}
                    >
                      <Text
                        className="text-sm font-medium"
                        style={{ color: colors.pierre.violet }}
                      >
                        {action.title}
                      </Text>
                    </TouchableOpacity>
                  ))}
                </View>
              ) : null}

              {/* Deep-link navigation button */}
              {hasRoute && (
                <TouchableOpacity
                  className="flex-row items-center justify-center mt-4 py-3 rounded-lg"
                  style={{ backgroundColor: colors.pierre.violet }}
                  onPress={() => {
                    onNavigate(notification);
                    onClose();
                  }}
                  testID="notification-detail-navigate"
                >
                  <ExternalLink size={16} color={colors.tokens.onPrimary} />
                  <Text
                    className="text-sm font-semibold ml-2"
                    style={{ color: colors.tokens.onPrimary }}
                  >
                    View Details
                  </Text>
                </TouchableOpacity>
              )}
            </View>
          </ScrollView>
        </View>
      </TouchableOpacity>
    </Modal>
  );
}
