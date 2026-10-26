// ABOUTME: About pane — the release and the help and legal links, as one Section of Rows
// ABOUTME: Row order comes from the shared settings declaration, so web lists the same three

import React from 'react';
import { View } from 'react-native';
import { PaneScrollView, Row, Section } from '../../components/ui';
import { useTranslation } from '@pierre/i18n';
import {
  APP_VERSION,
  HELP_URL,
  LEGAL_URL,
  settingsPane,
  settingsPaneSections,
} from '@pierre/shared-constants';
import { spacing, useThemeColors } from '../../constants/theme';
import { openExternal } from '../../utils/openExternal';

/**
 * What the app is, and where help and the legal documents live.
 *
 * Which model answers is an operator's reading, never an athlete's
 * (carnet#1147), so the pane names none.
 *
 * Three rows in one group: one fact that is read and two links that open the
 * browser, so only the links carry a chevron.
 */
export function AboutScreen() {
  const { t } = useTranslation();
  const colors = useThemeColors();

  const sections = settingsPaneSections('about');

  const renderRow = (section: string, index: number) => {
    const last = index === sections.length - 1;

    switch (section) {
      case 'version':
        return <Row key={section} compact last={last} title={t('app.version')} value={APP_VERSION} testID="about-section-version" />;

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
