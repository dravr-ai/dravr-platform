// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The shell's reconnect banner — names every connected provider flagged needs_reauth, on every tab, with the way to Connections
// ABOUTME: Reads the one provider-status query Home and Connections share; the warning tint under its bound ink, no shadow

import React, { useEffect, useRef } from 'react';
import { Pressable, Text, View } from 'react-native';
import { Feather } from '@expo/vector-icons';
import { usePathname, useRouter } from 'expo-router';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { useTranslation } from '@pierre/i18n';
import { useThemeColors } from '../constants/theme';
import { useProviderConnected } from '../hooks/useHome';
import { CONNECTIONS_ROUTE } from '../navigation/routes';

/** The path the Connections pane answers to, groups stripped as expo-router reports it. */
const CONNECTIONS_PATH = '/connections';

interface ReconnectBannerProps {
  /**
   * Whether the banner is the first thing under the status bar, and so pads
   * itself below it. False when another shell banner already sits above it.
   */
  insetTop: boolean;
}

/**
 * A connection flagged `needs_reauth` syncs nothing until the athlete signs
 * in again, and the only other place that says so is the Home activities
 * section — one athlete went three days without seeing it. The shell shows
 * it above every tab instead, for as long as the flag holds.
 *
 * The names come from {@link useProviderConnected}: connected providers only,
 * each display name once (the `sciotte` mirror and the `strava` OAuth row are
 * both "Strava"). Nothing is drawn until the status answers, when every
 * connection is healthy, or on the Connections pane itself, where each
 * flagged row already says it and the button would lead back to the same page.
 *
 * Connections keeps its own copy of the status, so the banner reads the
 * shared one again whenever the athlete leaves that pane.
 *
 * The text block is the live region, announced when the banner appears; the
 * button stays its own focus stop. The ground is opaque paper under a
 * `warning` tint, so the tab content never shows through, and the text takes
 * `on-warning-container`, the ink bound to that tint (DESIGN.md §2).
 */
export function ReconnectBanner({ insetTop }: ReconnectBannerProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const insets = useSafeAreaInsets();
  const router = useRouter();
  const pathname = usePathname();
  const { needsReconnect, refetch } = useProviderConnected();

  // Connections reads the status on its own, so leaving it is when a
  // reconnect made there has to reach the shared answer this banner draws.
  const previousPath = useRef(pathname);
  useEffect(() => {
    if (previousPath.current === CONNECTIONS_PATH && pathname !== CONNECTIONS_PATH) {
      void refetch();
    }
    previousPath.current = pathname;
  }, [pathname, refetch]);

  if (needsReconnect.length === 0 || pathname === CONNECTIONS_PATH) {
    return null;
  }

  // Joined with a comma, as Home and the chat header join provider names:
  // the phone's JavaScript engine ships no `Intl.ListFormat`.
  const providers = needsReconnect.join(', ');

  return (
    <View className="bg-background-primary" testID="reconnect-banner">
      <View
        className="bg-warning/15 border-b border-warning/40 px-4 pb-3 flex-row items-center"
        // The status bar's height is only known at runtime.
        style={{ paddingTop: (insetTop ? insets.top : 0) + 12 }}
      >
        <View
          className="flex-1 flex-row items-start mr-3"
          accessibilityRole="alert"
          accessibilityLiveRegion="polite"
          testID="reconnect-banner-message"
        >
          <View className="mt-0.5">
            <Feather name="alert-triangle" size={16} color={colors.ink.warning} />
          </View>
          <View className="flex-1 ml-2">
            <Text className="text-sm font-semibold text-on-warning-container">
              {t('providers.reconnectNeeded')}
            </Text>
            <Text className="text-sm text-on-warning-container" testID="reconnect-banner-providers">
              {t('home.activities.reconnect', { providers })}
            </Text>
          </View>
        </View>
        <Pressable
          onPress={() => router.push(CONNECTIONS_ROUTE)}
          accessibilityRole="button"
          accessibilityLabel={t('providers.reconnect')}
          className="min-h-11 px-3 justify-center rounded-lg border border-on-warning-container/40"
          testID="reconnect-banner-action"
        >
          <Text className="text-sm font-semibold text-on-warning-container">{t('providers.reconnect')}</Text>
        </Pressable>
      </View>
    </View>
  );
}
