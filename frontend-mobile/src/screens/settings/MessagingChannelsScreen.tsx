// ABOUTME: Messaging channels screen — view, link and unlink chat apps after onboarding
// ABOUTME: Mobile could link a channel during onboarding but never manage one afterwards

import React, { useCallback, useState } from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  ActivityIndicator,
  Alert,
  Linking,
  type ViewStyle,
} from 'react-native';
import { PaneScrollView } from '../../components/ui';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useRouter, useFocusEffect } from 'expo-router';
import { Feather } from '@expo/vector-icons';
import type { AvailableChannel, ChannelLink } from '@pierre/api-client';
import { spacing, useThemeColors } from '../../constants/theme';
import { messagingApi } from '../../services/api';
import { useTranslation } from '@pierre/i18n';

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
  const router = useRouter();
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
      setError(err instanceof Error ? err.message : t('app.failedLoadChannels'));
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
        const message = err instanceof Error ? err.message : t('app.failedLoadChannels');
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
                const message = err instanceof Error ? err.message : t('app.failedUnlinkChannel');
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

  const cardStyle: ViewStyle = {
    backgroundColor: colors.background.tertiary,
    borderWidth: 1,
    borderColor: colors.border.default,
    borderRadius: 16,
    overflow: 'hidden',
  };

  const rowStyle: ViewStyle = {
    flexDirection: 'row',
    alignItems: 'center',
    paddingVertical: 14,
    paddingHorizontal: 16,
  };

  return (
    <SafeAreaView style={{ flex: 1, backgroundColor: colors.background.primary }} edges={['top']} testID="messaging-channels-screen">
      <View style={{ flexDirection: 'row', alignItems: 'center', paddingHorizontal: spacing.md, paddingVertical: spacing.sm }}>
        <TouchableOpacity onPress={() => router.back()} testID="back-button" style={{ padding: 8, marginRight: 8 }}>
          <Feather name="arrow-left" size={24} color={colors.text.primary} />
        </TouchableOpacity>
        <Text style={{ fontSize: 20, fontWeight: '600', color: colors.text.primary }}>{t('app.messaging')}</Text>
      </View>

      {isLoading ? (
        <View style={{ flex: 1, alignItems: 'center', justifyContent: 'center' }}>
          <ActivityIndicator size="large" color={colors.pierre.violet} />
        </View>
      ) : error ? (
        <View style={{ flex: 1, alignItems: 'center', justifyContent: 'center', padding: spacing.lg, gap: 12 }}>
          <Text style={{ color: colors.text.secondary, textAlign: 'center' }} testID="messaging-error">{error}</Text>
          <TouchableOpacity onPress={() => { void load(); }} testID="messaging-retry" style={{ paddingHorizontal: 20, paddingVertical: 12, borderRadius: 12, backgroundColor: colors.background.tertiary }}>
            <Text style={{ color: colors.text.primary, fontWeight: '600' }}>{t('common.retry')}</Text>
          </TouchableOpacity>
        </View>
      ) : (
        <PaneScrollView contentContainerStyle={{ padding: spacing.md, gap: spacing.lg }}>
          <View>
            <Text style={{ fontSize: 18, fontWeight: '600', color: colors.text.primary, marginBottom: 12 }}>{t('app.linked')}</Text>
            <View style={cardStyle}>
              {links.length === 0 ? (
                <View style={{ padding: spacing.md }} testID="messaging-no-links">
                  <Text style={{ color: colors.text.tertiary }}>
                    {/* "Link one below" points at nothing when the tenant has
                        no channel configured, so that case says what is
                        actually true instead of giving an instruction the
                        athlete cannot follow. */}
                    {available.length === 0
                      ? t('app.noChatAppsAvailableYet')
                      : t('app.noChatAppsLinked')}
                  </Text>
                </View>
              ) : (
                links.map((link, index) => (
                  <View
                    key={link.channel}
                    style={[
                      rowStyle,
                      index < links.length - 1
                        ? { borderBottomWidth: 1, borderBottomColor: colors.border.subtle }
                        : {},
                    ]}
                    testID={`messaging-link-${link.channel}`}
                  >
                    <View style={{ flex: 1 }}>
                      <Text style={{ fontSize: 16, color: colors.text.primary }}>
                        {available.find((c) => c.channel === link.channel)?.display_name ?? link.channel}
                      </Text>
                      <Text style={{ fontSize: 14, color: colors.text.tertiary }}>
                        {link.display_name ?? link.channel_user_id}
                      </Text>
                    </View>
                    {busyChannel === link.channel ? (
                      <ActivityIndicator color={colors.text.tertiary} />
                    ) : (
                      <TouchableOpacity
                        onPress={() => handleUnlink(link)}
                        testID={`messaging-unlink-${link.channel}`}
                        style={{ paddingHorizontal: 12, paddingVertical: 8 }}
                      >
                        <Text style={{ color: colors.pierre.red, fontWeight: '600' }}>{t('app.unlink')}</Text>
                      </TouchableOpacity>
                    )}
                  </View>
                ))
              )}
            </View>
          </View>

          <View>
            <Text style={{ fontSize: 18, fontWeight: '600', color: colors.text.primary, marginBottom: 12 }}>{t('app.available')}</Text>
            <View style={cardStyle}>
              {unlinked.map((channel, index) => (
                <TouchableOpacity
                  key={channel.channel}
                  style={[
                    rowStyle,
                    index < unlinked.length - 1
                      ? { borderBottomWidth: 1, borderBottomColor: colors.border.subtle }
                      : {},
                  ]}
                  onPress={() => handleLink(channel)}
                  disabled={busyChannel === channel.channel}
                  testID={`messaging-link-add-${channel.channel}`}
                >
                  <Text style={{ flex: 1, fontSize: 16, color: colors.text.primary }}>{channel.display_name}</Text>
                  {busyChannel === channel.channel ? (
                    <ActivityIndicator color={colors.text.tertiary} />
                  ) : (
                    <Feather name="chevron-right" size={20} color={colors.text.tertiary} />
                  )}
                </TouchableOpacity>
              ))}
              {/* An empty list has two causes and they are not the same news.
                  Nothing configured for the tenant is not "you already linked
                  everything", and saying the second when the first is true is
                  how this screen came to contradict itself. */}
              {available.length === 0 ? (
                <View style={{ padding: spacing.md }} testID="messaging-none-configured">
                  <Text style={{ color: colors.text.tertiary }}>{t('app.noChatAppsConfigured')}</Text>
                </View>
              ) : unlinked.length === 0 ? (
                <View style={{ padding: spacing.md }} testID="messaging-all-linked">
                  <Text style={{ color: colors.text.tertiary }}>{t('app.everyChatAppLinked')}</Text>
                </View>
              ) : null}
            </View>
          </View>
        </PaneScrollView>
      )}
    </SafeAreaView>
  );
}
