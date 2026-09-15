// ABOUTME: About pane — the release, which model answers the athlete, and the help and legal links, as one Section of Rows
// ABOUTME: Row order comes from the shared settings declaration, so web lists the same four

import React from 'react';
import { Alert, Linking, View } from 'react-native';
import { PaneScrollView, Row, Section } from '../../components/ui';
import { useQuery } from '@tanstack/react-query';
import { useTranslation } from '@pierre/i18n';
import {
  APP_VERSION,
  HELP_URL,
  LEGAL_URL,
  QUERY_KEYS,
  settingsPane,
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
 *
 * Four rows in one group: two facts that are read and two links that open the
 * browser, so only the links carry a chevron.
 */
export function AboutScreen() {
  const { t } = useTranslation();
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

  const sections = settingsPaneSections('about');

  const renderRow = (section: string, index: number) => {
    const last = index === sections.length - 1;

    switch (section) {
      case 'version':
        return <Row key={section} compact last={last} title={t('app.version')} value={APP_VERSION} testID="about-section-version" />;

      case 'coach-model':
        // A model name is a name, not a figure, so it takes the hint slot
        // rather than the mono value slot; the hint carries the id the sweep
        // reads it by.
        return (
          <Row
            key={section}
            compact
            last={last}
            title={t('about.agentModel')}
            hint={coachModelLabel ?? t('about.agentModelUnknown')}
            hintTestID="about-coach-model-value"
            testID="about-section-coach-model"
          />
        );

      case 'help':
        return (
          <Row
            key={section}
            last={last}
            title={t('about.helpCenter')}
            subtitle={t('about.helpHint')}
            onPress={() => { void openExternal(HELP_URL, t); }}
            testID="about-section-help"
          />
        );

      case 'legal':
        return (
          <Row
            key={section}
            last={last}
            title={t('about.legalDocuments')}
            subtitle={t('about.legalHint')}
            onPress={() => { void openExternal(LEGAL_URL, t); }}
            testID="about-section-legal"
          />
        );

      default:
        return null;
    }
  };

  return (
    <View style={{ flex: 1, backgroundColor: colors.background.primary }} testID="about-screen">
      <PaneScrollView contentContainerStyle={{ paddingVertical: spacing.md }}>
        <Section title={t(settingsPane('about').nameKey)}>{sections.map(renderRow)}</Section>
      </PaneScrollView>
    </View>
  );
}
