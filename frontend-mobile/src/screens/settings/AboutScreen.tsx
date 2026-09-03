// ABOUTME: About pane — the release, which model answers the athlete, and the help and legal links
// ABOUTME: Section order comes from the shared settings declaration, so web lists the same four rows

import React from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  Alert,
  Linking,
  type ViewStyle,
} from 'react-native';
import { PaneScrollView } from '../../components/ui';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useRouter } from 'expo-router';
import { useQuery } from '@tanstack/react-query';
import { Feather } from '@expo/vector-icons';
import { useTranslation } from '@pierre/i18n';
import {
  APP_VERSION,
  HELP_URL,
  LEGAL_URL,
  QUERY_KEYS,
  settingsPaneSections,
} from '@pierre/shared-constants';
import { spacing, useThemeColors } from '../../constants/theme';
import { userApi } from '../../services/api';
import { useAuth } from '../../contexts/AuthContext';

/**
 * Open an external URL, telling the athlete when the device cannot.
 *
 * A bare `Linking.openURL` rejects on a device with no handler, and an
 * unhandled rejection here looks identical to the row doing nothing.
 */
async function openExternal(
  url: string,
  t: (key: string, opts?: Record<string, unknown>) => string,
): Promise<void> {
  try {
    await Linking.openURL(url);
  } catch {
    Alert.alert(t('app.couldNotOpenLink'), t('app.openInBrowserInstead', { url }));
  }
}

/**
 * What the app is, and who is answering.
 *
 * The model line is read-only on purpose: an athlete does not bring their own
 * provider key, so the fact of which model replies belongs beside the version
 * rather than beside a field that invites a credential.
 */
export function AboutScreen() {
  const { t } = useTranslation();
  const router = useRouter();
  const colors = useThemeColors();
  const { isAuthenticated } = useAuth();

  const { data: llmSettings } = useQuery({
    queryKey: QUERY_KEYS.llmSettings.list(),
    queryFn: () => userApi.getLlmSettings(),
    enabled: isAuthenticated,
  });
  const systemProvider = llmSettings?.system_provider;
  const coachModelLabel = systemProvider
    ? [systemProvider.display_name, systemProvider.model].filter(Boolean).join(' · ')
    : null;

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

  const iconBox = (name: React.ComponentProps<typeof Feather>['name']) => (
    <View
      style={{
        width: 40,
        height: 40,
        borderRadius: 12,
        backgroundColor: colors.background.secondary,
        alignItems: 'center',
        justifyContent: 'center',
        marginRight: 12,
      }}
    >
      <Feather name={name} size={20} color={colors.text.secondary} />
    </View>
  );

  const sections = settingsPaneSections('about');

  const renderSection = (section: string, index: number) => {
    const divider =
      index < sections.length - 1
        ? { borderBottomWidth: 1, borderBottomColor: colors.border.subtle }
        : {};

    switch (section) {
      case 'version':
        return (
          <View key={section} style={[rowStyle, divider]} testID="about-section-version">
            {iconBox('info')}
            <View style={{ flex: 1 }}>
              <Text style={{ fontSize: 16, color: colors.text.primary }}>{t('app.version')}</Text>
              <Text style={{ fontSize: 14, color: colors.text.tertiary }}>{APP_VERSION}</Text>
            </View>
          </View>
        );

      case 'coach-model':
        return (
          <View key={section} style={[rowStyle, divider]} testID="about-section-coach-model">
            {iconBox('cpu')}
            <View style={{ flex: 1 }}>
              <Text style={{ fontSize: 16, color: colors.text.primary }}>{t('about.coachModel')}</Text>
              <Text
                style={{ fontSize: 14, color: colors.text.tertiary }}
                testID="about-coach-model-value"
              >
                {coachModelLabel ?? t('about.coachModelUnknown')}
              </Text>
            </View>
          </View>
        );

      case 'help':
        return (
          <TouchableOpacity
            key={section}
            style={[rowStyle, divider]}
            onPress={() => { void openExternal(HELP_URL, t); }}
            testID="about-section-help"
          >
            {iconBox('help-circle')}
            <View style={{ flex: 1 }}>
              <Text style={{ fontSize: 16, color: colors.text.primary }}>{t('about.helpCenter')}</Text>
              <Text style={{ fontSize: 14, color: colors.text.tertiary }}>{t('about.helpHint')}</Text>
            </View>
            <Feather name="chevron-right" size={20} color={colors.text.tertiary} />
          </TouchableOpacity>
        );

      case 'legal':
        return (
          <TouchableOpacity
            key={section}
            style={[rowStyle, divider]}
            onPress={() => { void openExternal(LEGAL_URL, t); }}
            testID="about-section-legal"
          >
            {iconBox('file-text')}
            <View style={{ flex: 1 }}>
              <Text style={{ fontSize: 16, color: colors.text.primary }}>{t('about.legalDocuments')}</Text>
              <Text style={{ fontSize: 14, color: colors.text.tertiary }}>{t('about.legalHint')}</Text>
            </View>
            <Feather name="chevron-right" size={20} color={colors.text.tertiary} />
          </TouchableOpacity>
        );

      default:
        return null;
    }
  };

  return (
    <SafeAreaView
      style={{ flex: 1, backgroundColor: colors.background.primary }}
      edges={['top']}
      testID="about-screen"
    >
      <View style={{ flexDirection: 'row', alignItems: 'center', paddingHorizontal: spacing.md, paddingVertical: spacing.sm }}>
        <TouchableOpacity onPress={() => router.back()} testID="back-button" style={{ padding: 8, marginRight: 8 }}>
          <Feather name="arrow-left" size={24} color={colors.text.primary} />
        </TouchableOpacity>
        <Text style={{ fontSize: 20, fontWeight: '600', color: colors.text.primary }}>{t('about.title')}</Text>
      </View>

      <PaneScrollView contentContainerStyle={{ padding: spacing.md }}>
        <View style={cardStyle}>{sections.map(renderSection)}</View>
      </PaneScrollView>
    </SafeAreaView>
  );
}
