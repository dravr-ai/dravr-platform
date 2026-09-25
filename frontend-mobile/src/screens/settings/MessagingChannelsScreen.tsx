// ABOUTME: Messaging channels pane — two Sections, the chat apps linked to the account and the ones still available to link
// ABOUTME: Unlink and connect are ink words on the trailing side of a 52 row; no card, no filled button (Boreal v2.2 Phase 4)

import React, { useCallback, useState } from 'react';
import { View, Text, ActivityIndicator, Alert, Linking } from 'react-native';
import { EmptyState, PaneScrollView, Row, Section } from '../../components/ui';
import { useFocusEffect } from 'expo-router';
import type { AvailableChannel, ChannelLink } from '@pierre/api-client';
import { spacing, useThemeColors } from '../../constants/theme';
import { messagingApi } from '../../services/api';
import { useTranslation } from '@pierre/i18n';
import { describeApiError } from '@pierre/ui-logic';

/**
 * Manage which chat apps are linked to the account.
 *
 * Onboarding could link a channel, but nothing in the app could show what was
 * linked or unlink it afterwards — a one-way door. This is the mobile
 * counterpart of the web Settings messaging surface, built on the same shared
 * `listLinks` / `initLink` / `deleteLink` calls onboarding already uses.
 */
export function MessagingChannelsScreen() {
  const { t } = useTranslation();
  const colors = useThemeColors();

  const [links, setLinks] = useState<ChannelLink[]>([]);
  const [available, setAvailable] = useState<AvailableChannel[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [busyChannel, setBusyChannel] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setIsLoading(true);
      setError(null);
      const [linked, channels] = await Promise.all([
        messagingApi.listLinks(),
        messagingApi.getAvailableChannels(),
      ]);
      setLinks(linked);
      setAvailable(channels);
    } catch (err) {
      // Surface the failure rather than rendering an empty list, which would
      // read as "no channels linked" and invite someone to re-link one they
      // already have.
      setError(describeApiError(err, { t, fallbackKey: 'app.failedLoadChannels' }));
    } finally {
      setIsLoading(false);
    }
  }, [t]);

  useFocusEffect(
    useCallback(() => {
      void load();
    }, [load]),
  );

  const linkedChannels = new Set(links.map((l) => l.channel));
  const unlinked = available.filter((channel) => !linkedChannels.has(channel.channel));

  /**
   * Start the link and hand the athlete to the chat app.
   *
   * `initLink` returns the provider's own URL carrying the pairing code — a
   * `t.me` or `wa.me` address — so opening it lands in the installed app with
   * the code already in the message box. That is the path the onboarding
   * screen's Open button takes; settings takes it too rather than only
   * listing what somebody linked elsewhere. The link lands server-side, and
   * the focus effect reloads the list when the athlete comes back.
   */
  const handleLink = (channel: AvailableChannel) => {
    void (async () => {
      setBusyChannel(channel.channel);
      try {
        const link = await messagingApi.initLink(channel.channel);
        await Linking.openURL(link.linking_url);
      } catch (err) {
        const message = describeApiError(err, { t, fallbackKey: 'app.failedLoadChannels' });
        Alert.alert(
          t('app.couldNotStartConnection', { channel: channel.display_name }),
          message,
        );
      } finally {
        setBusyChannel(null);
      }
    })();
  };

  const handleUnlink = (link: ChannelLink) => {
    const name = available.find((c) => c.channel === link.channel)?.display_name ?? link.channel;
    Alert.alert(
      t('app.confirmUnlinkChannel', { channel: name }),
      t('app.unlinkChannelWarning'),
      [
        { text: t('common.cancel'), style: 'cancel' },
        {
          text: t('app.unlink'),
          style: 'destructive',
          onPress: () => {
            void (async () => {
              setBusyChannel(link.channel);
              try {
                await messagingApi.deleteLink(link.channel);
                await load();
              } catch (err) {
                const message = describeApiError(err, { t, fallbackKey: 'app.failedUnlinkChannel' });
                Alert.alert(t('app.couldNotUnlink'), message);
              } finally {
                setBusyChannel(null);
              }
            })();
          },
        },
      ],
    );
  };

  /** The ink word on a row's trailing side, or the spinner while that channel is busy. */
  const inkAction = (channel: string, label: string, onPress: () => void, testID: string) =>
    busyChannel === channel ? (
      <ActivityIndicator color={colors.text.tertiary} />
    ) : (
      <Text
        className="text-sm font-medium text-primary"
        accessibilityRole="button"
        onPress={onPress}
        testID={testID}
      >
        {label}
      </Text>
    );

  return (
    <View className="flex-1 bg-background-primary" testID="messaging-channels-screen">
      {isLoading ? (
        <View className="flex-1 items-center justify-center">
          <ActivityIndicator size="large" color={colors.text.primary} />
        </View>
      ) : error ? (
        // The retry is a sibling, not a nested span: Android gives a nested
        // `Text` no native view, so a tap on it would reach nothing there.
        <View className="flex-row flex-wrap items-baseline px-4 py-3">
          <Text className="text-sm text-error" testID="messaging-error">{error}</Text>
          <Text
            className="text-sm text-primary font-medium ml-1"
            accessibilityRole="button"
            onPress={() => { void load(); }}
            testID="messaging-retry"
          >
            {t('common.retry')}
          </Text>
        </View>
      ) : (
        <PaneScrollView contentContainerStyle={{ paddingTop: spacing.lg, paddingBottom: spacing.xl }}>
          <View className="gap-8">
            <Section title={t('app.linked')} testID="messaging-linked-section">
              {links.length === 0 ? (
                <EmptyState testID="messaging-no-links">
                  {/* "Link one below" points at nothing when the tenant has no
                      channel configured, so that case says what is actually
                      true instead of giving an instruction the athlete cannot
                      follow. */}
                  {available.length === 0 ? t('app.noChatAppsAvailableYet') : t('app.noChatAppsLinked')}
                </EmptyState>
              ) : (
                links.map((link, index) => (
                  <Row
                    key={link.channel}
                    testID={`messaging-link-${link.channel}`}
                    title={available.find((c) => c.channel === link.channel)?.display_name ?? link.channel}
                    hint={link.display_name ?? link.channel_user_id}
                    last={index === links.length - 1}
                    trailing={inkAction(
                      link.channel,
                      t('app.unlink'),
                      () => handleUnlink(link),
                      `messaging-unlink-${link.channel}`,
                    )}
                  />
                ))
              )}
            </Section>

            <Section title={t('app.available')} testID="messaging-available-section">
              {/* An empty list has two causes and they are not the same news.
                  Nothing configured for the tenant is not "you already linked
                  everything", and saying the second when the first is true is
                  how this screen came to contradict itself. */}
              {available.length === 0 ? (
                <EmptyState testID="messaging-none-configured">{t('app.noChatAppsConfigured')}</EmptyState>
              ) : unlinked.length === 0 ? (
                <EmptyState testID="messaging-all-linked">{t('app.everyChatAppLinked')}</EmptyState>
              ) : (
                unlinked.map((channel, index) => (
                  <Row
                    key={channel.channel}
                    testID={`messaging-link-add-${channel.channel}`}
                    title={channel.display_name}
                    last={index === unlinked.length - 1}
                    onPress={busyChannel === channel.channel ? undefined : () => handleLink(channel)}
                    // The whole 52 row is the target; the ink word only says what the tap does.
                    trailing={
                      busyChannel === channel.channel ? (
                        <ActivityIndicator color={colors.text.tertiary} />
                      ) : (
                        <Text className="text-sm font-medium text-primary">{t('app.connect')}</Text>
                      )
                    }
                  />
                ))
              )}
            </Section>
          </View>
        </PaneScrollView>
      )}
    </View>
  );
}
