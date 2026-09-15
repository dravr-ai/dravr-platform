// ABOUTME: Profile pane — the identity row, then display name, account email, appearance and app language as Sections
// ABOUTME: Holds what the web Profile pane holds, so the two clients group the same things

import React, { useState } from 'react';
import { Alert, Text, View } from 'react-native';
import { useRouter } from 'expo-router';
import { Feather } from '@expo/vector-icons';
import { avatarSlot, initialsFor } from '@pierre/chat-utils';
import { spacing, useThemeColors, useTheme } from '../../constants/theme';
import type { AppearancePref } from '../../hooks/useAppearancePref';
import { Button, InitialsAvatar, Input, PaneScrollView, Row, Section } from '../../components/ui';
import { LanguageSwitcher } from '../../components/LanguageSwitcher';
import { userApi } from '../../services/api';
import { useAuth } from '../../contexts/AuthContext';
import { useTranslation } from '@pierre/i18n';

/** The three appearance choices, in the order the pane lists them. */
const APPEARANCE_OPTIONS: readonly AppearancePref[] = ['system', 'dark', 'light'];

/**
 * Edit the account profile.
 *
 * The Settings screen previously offered "Edit Profile" and "Personal
 * Information" as two separate rows, neither of which was wired to anything —
 * both promised a destination that did not exist. They now both lead here, and
 * this screen mirrors the web Settings t('common.profile') tab field for field so the two
 * platforms cannot drift: display name is editable, email is shown read-only
 * because it is the account identifier and is not changeable from this surface.
 *
 * The identity block is the same 56 row the settings root draws — the initials
 * circle at 40, name and email beside it — rather than a hero ring; the rest
 * is `Section`s of fields and `Row`s on the ground (DESIGN.md §10).
 */
export function ProfileScreen() {
  const { t } = useTranslation();
  const router = useRouter();
  const colors = useThemeColors();
  const { pref: appearancePref, setPref: setAppearancePref } = useTheme();
  const { user, updateUser } = useAuth();

  const currentName = user?.display_name ?? '';
  const [displayName, setDisplayName] = useState(currentName);
  const [isSaving, setIsSaving] = useState(false);

  // Same rule the web Save button uses: nothing to save until the value differs.
  const trimmed = displayName.trim();
  const isDirty = trimmed.length > 0 && trimmed !== currentName;

  const handleSave = async () => {
    if (!isDirty) {
      return;
    }
    setIsSaving(true);
    try {
      const response = await userApi.updateProfile({ display_name: trimmed });
      // Reflect the saved value locally so the Settings header and avatar
      // initial update without a round-trip through a refetch.
      await updateUser({ display_name: response.user?.display_name ?? trimmed });
      Alert.alert(t('app.profileUpdated'), t('app.displayNameSaved'));
      router.back();
    } catch (err) {
      const message = err instanceof Error ? err.message : t('app.failedToUpdateProfile');
      Alert.alert(t('app.couldNotSaveProfile'), message);
    } finally {
      setIsSaving(false);
    }
  };

  const shownName = user?.display_name || user?.email?.split('@')[0] || t('app.athlete');

  const appearanceLabel = (option: AppearancePref): string =>
    option === 'system'
      ? t('settings.appearanceSystem')
      : option === 'dark'
        ? t('settings.appearanceDark')
        : t('settings.appearanceLight');

  const appearanceHint = (option: AppearancePref): string =>
    option === 'system'
      ? t('settings.appearanceSystemHint')
      : option === 'dark'
        ? t('settings.appearanceDarkHint')
        : t('settings.appearanceLightHint');

  return (
    <View style={{ flex: 1, backgroundColor: colors.background.primary }} testID="profile-screen">
      <PaneScrollView contentContainerStyle={{ paddingVertical: spacing.md }}>
        {/* The identity row, as the settings root draws it. */}
        <View className="h-14 flex-row items-center mb-8 px-4">
          <InitialsAvatar
            initials={initialsFor(shownName)}
            slot={avatarSlot({ id: user?.id ?? shownName, agent_id: null, group_id: null })}
            size={40}
            testID="profile-avatar"
          />
          <View className="flex-1 ml-3">
            <Text className="text-base font-semibold text-text-primary" numberOfLines={1}>
              {shownName}
            </Text>
            <Text className="text-sm text-text-secondary" numberOfLines={1}>
              {user?.email}
            </Text>
          </View>
        </View>

        <View className="gap-8">
          <Section title={t('common.profile')}>
            {/* The field pays the pane's inset itself; the row under it carries its own. */}
            <View className="px-4">
              <Input
                label={t('app.displayName')}
                value={displayName}
                onChangeText={setDisplayName}
                placeholder={t('app.howAgentAddressesYou')}
                autoCapitalize="words"
                testID="profile-display-name-input"
              />
            </View>
            {/* Read-only: the email identifies the account and is not editable here. */}
            <Row compact last title={t('common.email')} value={user?.email ?? ''} testID="profile-email-readonly" />
          </Section>

          {/* Appearance and language sit with the profile on web too — they are
              how the athlete's own copy of the app reads, not a pane of their
              own. */}
          <Section title={t('settings.appearance')} testID="profile-appearance-section">
            {APPEARANCE_OPTIONS.map((option, index) => {
              const isSelected = appearancePref === option;
              return (
                <Row
                  key={option}
                  title={appearanceLabel(option)}
                  subtitle={appearanceHint(option)}
                  onPress={() => { void setAppearancePref(option); }}
                  showChevron={false}
                  last={index === APPEARANCE_OPTIONS.length - 1}
                  accessibilityRole="radio"
                  accessibilityState={{ selected: isSelected }}
                  trailing={
                    isSelected ? <Feather name="check" size={18} color={colors.tokens.primary} /> : undefined
                  }
                  testID={`appearance-option-${option}`}
                />
              );
            })}
          </Section>

          {/* The switcher sets the chrome language AND `users.locale`, so the
              coach answers in the language the athlete reads the app in. */}
          <Section
            title={t('settings.language')}
            description={t('settings.languageDescription')}
            testID="profile-language-section"
          >
            <LanguageSwitcher serverLocale={user?.locale} />
          </Section>

          <View className="px-4">
            <Button
              title={t('app.saveChanges')}
              variant="primary"
              onPress={() => { void handleSave(); }}
              disabled={!isDirty}
              loading={isSaving}
              testID="profile-save-button"
            />
          </View>
        </View>
      </PaneScrollView>
    </View>
  );
}
