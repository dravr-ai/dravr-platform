// ABOUTME: Agent Store detail screen showing full agent info with install/uninstall/edit actions
// ABOUTME: Installing puts the agent on the athlete's team; Edit forks a copy first, Open chat binds the installed agent

import React, { useState, useCallback, useEffect, useRef } from 'react';
import {
  View,
  Text,
  ScrollView,
  TouchableOpacity,
  ActivityIndicator,
  Alert,
  StyleSheet,
} from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { Stack, useRouter, useLocalSearchParams } from 'expo-router';
import { useCardStyle, useThemeColors, categoryAccent, categoryInk } from '../../constants/theme';
import { Feather } from '@expo/vector-icons';
import { coachesApi, storeApi } from '../../services/api';
import { trackMobile } from '../../services/analytics';
import { COACH_EDIT_ROUTE, threadHref } from '../../navigation/routes';
import { useOpenPersonalTopic } from '../chat/usePersonalThread';
import { useAuth } from '../../contexts/AuthContext';
import { PostInstallHint } from './PostInstallHint';
import { Section, Row } from '../../components/ui';
import type { StoreAgent, StoreAgentDetail } from '../../types';
import { useTranslation } from '@pierre/i18n';
import { formatCount, formatDate } from '@pierre/chat-utils';
import { coachCategoryLabelKey, countKey, INSTALL_COUNT_KEYS } from '@pierre/shared-constants';

/**
 * The athlete's installation of a listing, if any. The installations endpoint
 * returns two shapes: a membership on a catalogue agent is the listing itself
 * (same id), and a copy — forked when the athlete edited it, or installed from
 * another workspace — has its own id. Both carry the listing's handle, so the
 * handle is what maps a listing back to the entry uninstall and edit address.
 */
function findInstallation(listing: StoreAgentDetail, installations: StoreAgent[]): StoreAgent | null {
  if (!listing.handle) return null;
  return installations.find((entry) => entry.handle === listing.handle) ?? null;
}

/**
 * The agent the install put on the athlete's team: its id binds the thread
 * Open chat starts, its title and handle are what the post-install hint teaches.
 */
interface InstalledAgent {
  id: string;
  title: string;
  handle: string | undefined;
}
export function StoreCoachDetailScreen() {
  const { t, language } = useTranslation();
  const colors = useThemeColors();
  // Still used by the action bar's Edit/Installed buttons below — the two
  // stacked prompt/system-prompt/metadata blocks lost their card fill per
  // Boreal v2.2 Phase 5 (P5.3), but the action bar itself is out of that scope.
  const cardStyle = useCardStyle();
  const router = useRouter();
  const openPersonalTopic = useOpenPersonalTopic();
  const { agentId } = useLocalSearchParams<{ agentId: string }>();
  const { isAuthenticated } = useAuth();
  // The action bar sits in the flow above the system tab bar; its own padding
  // keeps the buttons clear of the home indicator on a phone without a bar.
  const actionBarBottom = Math.max(useSafeAreaInsets().bottom, 12);
  const [coach, setCoach] = useState<StoreAgentDetail | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [isInstalling, setIsInstalling] = useState(false);
  const [installation, setInstallation] = useState<StoreAgent | null>(null);
  const [postInstall, setPostInstall] = useState<InstalledAgent | null>(null);
  const [isForking, setIsForking] = useState(false);
  // A second tap on Edit lands before the re-render that disables it.
  const forkingRef = useRef(false);
  const [isOpeningChat, setIsOpeningChat] = useState(false);
  // A second tap lands before the re-render that disables the button.
  const openingChatRef = useRef(false);
  const isInstalled = installation !== null;

  const loadCoachDetail = useCallback(async () => {
    if (!isAuthenticated || !agentId) return;

    try {
      setIsLoading(true);
      const response = await storeApi.get(agentId);
      setCoach(response);

      // Already installed? The entry is what uninstall and edit address.
      const installations = await storeApi.getInstallations();
      setInstallation(findInstallation(response, installations.agents));
    } catch (error) {
      console.error('Failed to load coach detail:', error);
      Alert.alert(t('common.error'), t('app.failedLoadAgentDetails'));
    } finally {
      setIsLoading(false);
    }
  }, [isAuthenticated, agentId, t]);

  useEffect(() => {
    loadCoachDetail();
  }, [loadCoachDetail]);

  const handleInstall = async () => {
    if (!coach) return;

    try {
      setIsInstalling(true);
      const response = await storeApi.install(coach.id);
      setInstallation(response.agent);
      setPostInstall({ id: response.agent.id, title: coach.title, handle: response.agent.handle });
      trackMobile({ name: 'feature_engaged', props: { feature: 'coach_installed' } });
    } catch (error) {
      console.error('Failed to install coach:', error);
      Alert.alert(t('common.error'), t('app.failedInstallAgent'));
    } finally {
      setIsInstalling(false);
    }
  };

  // t('discover.openChat') on the post-install hint: a new topic in the
  // athlete's personal thread, bound to the agent the install returned — the
  // catalogue agent itself for a membership, the copy for another workspace's
  // listing. Binding it with the topic is what makes the server post the
  // agent's welcome before the thread is read.
  const handleOpenChat = async () => {
    if (!postInstall || openingChatRef.current) return;
    openingChatRef.current = true;
    setIsOpeningChat(true);
    try {
      const personalThreadId = await openPersonalTopic({ agent_id: postInstall.id });
      setPostInstall(null);
      router.push(threadHref(personalThreadId));
    } catch (error) {
      console.error('Failed to open a chat with the installed agent:', error);
      Alert.alert(t('common.error'), t('app.couldNotOpenChat'));
    } finally {
      openingChatRef.current = false;
      setIsOpeningChat(false);
    }
  };

  // A membership points at the catalogue agent, which is not the athlete's to
  // edit: the server forks a copy of it onto their team first, and the editor
  // opens on that copy. An installation that is already a copy opens as is.
  const handleEdit = async () => {
    if (!coach || !installation || forkingRef.current) return;
    if (installation.id !== coach.id) {
      router.push({ pathname: COACH_EDIT_ROUTE, params: { agentId: installation.id } });
      return;
    }
    forkingRef.current = true;
    setIsForking(true);
    try {
      const copy = await coachesApi.fork(coach.id);
      // From here on the screen addresses the copy: uninstall removes it.
      setInstallation({ ...installation, id: copy.id });
      router.push({ pathname: COACH_EDIT_ROUTE, params: { agentId: copy.id } });
    } catch (error) {
      console.error('Failed to fork the agent for editing:', error);
      Alert.alert(t('common.error'), t('app.failedUpdateAgent'));
    } finally {
      forkingRef.current = false;
      setIsForking(false);
    }
  };

  const handleUninstall = async () => {
    if (!coach || !installation) return;
    // The listing's id for a membership, the copy's id for a copy: the server
    // ends the one and deletes the other.
    const installationId = installation.id;

    Alert.alert(
      t('app.uninstallAgentQ'),
      t('app.confirmUninstallAgent', { coach: coach.title }),
      [
        { text: t('common.cancel'), style: 'cancel' },
        {
          text: t('app.uninstall'),
          style: 'destructive',
          onPress: async () => {
            try {
              setIsInstalling(true);
              await storeApi.uninstall(installationId);
              setInstallation(null);
              setPostInstall(null);
              Alert.alert(t('app.uninstalled'), t('app.agentRemovedFromLibrary'));
            } catch (error) {
              console.error('Failed to uninstall coach:', error);
              Alert.alert(t('common.error'), t('app.failedUninstallAgent'));
            } finally {
              setIsInstalling(false);
            }
          },
        },
      ]
    );
  };

  if (isLoading) {
    return (
      <View className="flex-1 bg-background-primary">
        <View className="flex-1 justify-center items-center">
          <ActivityIndicator size="large" color={colors.tokens.primary} />
          <Text className="mt-3 text-text-secondary text-base">{t('discover.loadingAgentDetails')}</Text>
        </View>
      </View>
    );
  }

  if (!coach) {
    return (
      <View className="flex-1 bg-background-primary">
        <View className="flex-1 justify-center items-center p-6">
          <Text className="text-lg text-text-secondary mb-3">{t('discover.agentNotFound')}</Text>
          <TouchableOpacity
            className="px-5 py-2 bg-primary rounded-lg"
            onPress={() => router.push('/(app)/(tabs)/(discover)')}
          >
            <Text className="text-on-primary text-base font-medium">{t('discover.goBack')}</Text>
          </TouchableOpacity>
        </View>
      </View>
    );
  }

  // The category's hue as a pair: the accent fills tints, rules and borders,
  // and every label drawn on one of those tints takes the ink bound to it.
  // The accent drawn as text on a tint of itself does not clear AA in light.
  const categoryFill = categoryAccent(colors, coach.category);
  const categoryLabelInk = categoryInk(colors, coach.category);

  return (
    <View className="flex-1 bg-background-primary" testID="store-coach-detail-screen">
      {/* The native header names the agent and carries the way back. */}
      <Stack.Screen options={{ title: coach.title }} />

      <ScrollView
        className="flex-1"
        contentInsetAdjustmentBehavior="automatic"
        showsVerticalScrollIndicator={false}
      >
        {/* Category & Stats */}
        <View className="flex-row justify-between items-center px-4 pt-4 pb-2">
          <View
            testID="category-badge"
            className="px-3 py-1 rounded-full"
            style={{ backgroundColor: categoryFill + '20' }}
          >
            <Text className="text-sm font-semibold" style={{ color: categoryLabelInk }}>
              {t(coachCategoryLabelKey(coach.category))}
            </Text>
          </View>
          <Text testID="install-count" className="text-sm text-text-secondary">
            {t(countKey(coach.install_count, language, INSTALL_COUNT_KEYS), { count: coach.install_count })}
          </Text>
        </View>

        {/* Title */}
        <Text testID="coach-title" className="text-2xl font-bold text-text-primary px-4 mb-2">{coach.title}</Text>

        {/* Description */}
        {coach.description && (
          <Text className="text-base text-text-secondary px-4 leading-[22px] mb-3">{coach.description}</Text>
        )}

        {/* Tags */}
        {coach.tags.length > 0 && (
          <View className="px-4 py-3">
            <Text className="text-sm font-semibold text-text-secondary mb-2">{t('discover.tagsSection')}</Text>
            <View className="flex-row flex-wrap">
              {coach.tags.map((tag) => (
                <View
                  key={tag}
                  className="px-3 py-1.5 rounded-full mr-2 mb-2"
                  style={{
                    backgroundColor: `${categoryFill}15`,
                    borderWidth: 1,
                    borderColor: `${categoryFill}30`,
                  }}
                >
                  <Text className="text-sm font-medium" style={{ color: categoryLabelInk }}>{tag}</Text>
                </View>
              ))}
            </View>
          </View>
        )}

        {/* Sample Prompts, as a hairline list — no card, no per-item radius. */}
        {coach.sample_prompts.length > 0 && (
          <View className="mt-2">
            <Section title={t('discover.samplePrompts')}>
              {coach.sample_prompts.map((prompt, index) => (
                <View
                  key={prompt}
                  className={`px-4 py-3 ${index < coach.sample_prompts.length - 1 ? 'border-b border-border-faint' : ''}`}
                  style={index < coach.sample_prompts.length - 1 ? { borderBottomWidth: StyleSheet.hairlineWidth } : undefined}
                >
                  <Text className="text-base text-text-primary leading-5">{prompt}</Text>
                </View>
              ))}
            </Section>
          </View>
        )}

        {/* System Prompt Preview */}
        <View className="mt-8">
          <Section title={t('app.systemPrompt')}>
            <View className="px-4">
              <Text className="text-sm text-text-secondary leading-5 font-mono" numberOfLines={10}>
                {coach.system_prompt}
              </Text>
              {coach.system_prompt.length > 500 && (
                <Text className="text-xs text-text-tertiary italic mt-2">
                  ...and more ({coach.token_count} tokens)
                </Text>
              )}
            </View>
          </Section>
        </View>

        {/* Metadata */}
        <View className="mt-8">
          <Section title={t('discover.detailsSection')}>
            <Row title={t('discover.tokenCount')} value={formatCount(coach.token_count, language)} compact last={!coach.published_at} />
            {coach.published_at && (
              <Row
                title={t('discover.publishedOn')}
                value={formatDate(coach.published_at, language)}
                compact
                last
              />
            )}
          </Section>
        </View>

        {/* Scrolls the last section clear of the action bar below. */}
        <View style={{ height: 96 }} />
      </ScrollView>

      {/* Post-install hint + Install/Uninstall/Edit actions, in the flow above
          the system tab bar: the screen ends where the bar begins. */}
      <View
        className="bg-background-primary border-t border-border-faint p-3"
        style={{ paddingBottom: actionBarBottom }}
        testID="coach-detail-action-bar"
      >
        {postInstall && (
          <View className="mb-3">
            <PostInstallHint
              agentTitle={postInstall.title}
              handle={postInstall.handle}
              isOpeningChat={isOpeningChat}
              onOpenChat={handleOpenChat}
              onDismiss={() => setPostInstall(null)}
            />
          </View>
        )}
        {/* The author's own listing offers no action: the agent is theirs,
            not a Store install, so there is nothing to install or remove. */}
        {coach.is_own ? null : isInstalled ? (
          <View className="flex-row gap-3">
            {/* An installed listing is editable from here: a membership forks a copy first. */}
            <TouchableOpacity
              className="flex-1 flex-row items-center justify-center py-3.5 rounded-xl"
              style={{
                ...cardStyle,
                borderRadius: 12,
                borderColor: colors.border.strong,
              }}
              onPress={handleEdit}
              disabled={isInstalling || isForking}
              accessibilityRole="button"
              accessibilityLabel={t('app.editAgent')}
              testID="edit-coach-button"
            >
              <Feather name="edit-2" size={18} color={colors.pierre.violet} />
              <Text className="text-text-primary text-base font-medium ml-2">{t('app.editAgent')}</Text>
            </TouchableOpacity>
            <TouchableOpacity
              className="flex-1 flex-row items-center justify-center py-3.5 rounded-xl"
              style={{
                ...cardStyle,
                borderRadius: 12,
                borderColor: colors.border.strong,
              }}
              onPress={handleUninstall}
              disabled={isInstalling || isForking}
              testID="installed-button"
            >
              {isInstalling ? (
                <ActivityIndicator size="small" color={colors.text.primary} />
              ) : (
                <>
                  <Feather name="check" size={18} color={colors.pierre.activity} />
                  <Text className="text-text-primary text-base font-medium ml-2">{t('app.installed')}</Text>
                </>
              )}
            </TouchableOpacity>
          </View>
        ) : (
          <TouchableOpacity
            className="flex-row items-center justify-center py-3.5 rounded-xl"
            style={{
              backgroundColor: colors.pierre.violet,
            }}
            onPress={handleInstall}
            disabled={isInstalling}
          >
            {isInstalling ? (
              <ActivityIndicator size="small" color={colors.tokens.onPrimary} />
            ) : (
              <>
                <Feather name="download" size={18} color={colors.tokens.onPrimary} />
                <Text className="text-base font-semibold ml-2" style={{ color: colors.tokens.onPrimary }}>{t('app.installAgent')}</Text>
              </>
            )}
          </TouchableOpacity>
        )}
      </View>
    </View>
  );
}

