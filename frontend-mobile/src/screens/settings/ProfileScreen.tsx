// ABOUTME: Profile pane — display name, account email, appearance and app language
// ABOUTME: Holds what the web Profile pane holds, so the two clients group the same things

import React, { useState } from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  ActivityIndicator,
  Alert,
  type ViewStyle,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useRouter } from 'expo-router';
import { Feather } from '@expo/vector-icons';
import { spacing, useThemeColors, useTheme } from '../../constants/theme';
import type { AppearancePref } from '../../hooks/useAppearancePref';
import { Input, PaneScrollView } from '../../components/ui';
import { LanguageSwitcher } from '../../components/LanguageSwitcher';
import { userApi } from '../../services/api';
import { useAuth } from '../../contexts/AuthContext';
import { useTranslation } from '@pierre/i18n';

/**
 * Edit the account profile.
 *
 * The Settings screen previously offered "Edit Profile" and "Personal
 * Information" as two separate rows, neither of which was wired to anything —
 * both promised a destination that did not exist. They now both lead here, and
 * this screen mirrors the web Settings t('common.profile') tab field for field so the two
 * platforms cannot drift: display name is editable, email is shown read-only
 * because it is the account identifier and is not changeable from this surface.
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

  const cardStyle: ViewStyle = {
    backgroundColor: colors.background.tertiary,
    borderWidth: 1,
    borderColor: colors.border.default,
    borderRadius: 16,
    padding: 16,
  };

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

  const initial = (user?.display_name || user?.email || '?').charAt(0).toUpperCase();

  return (
    <SafeAreaView style={{ flex: 1, backgroundColor: colors.background.primary }} edges={['top']} testID="profile-screen">
      <View style={{ flexDirection: 'row', alignItems: 'center', paddingHorizontal: spacing.md, paddingVertical: spacing.sm }}>
        <TouchableOpacity onPress={() => router.back()} testID="back-button" style={{ padding: 8, marginRight: 8 }}>
          <Feather name="arrow-left" size={24} color={colors.text.primary} />
        </TouchableOpacity>
        <Text style={{ fontSize: 20, fontWeight: '600', color: colors.text.primary }}>{t('common.profile')}</Text>
      </View>

      <PaneScrollView contentContainerStyle={{ padding: spacing.md, gap: spacing.lg }}>
        <View style={{ alignItems: 'center', gap: 8 }}>
          <View
            style={{
              width: 88,
              height: 88,
              borderRadius: 44,
              borderWidth: 2,
              borderColor: colors.tokens.primary,
              alignItems: 'center',
              justifyContent: 'center',
            }}
            testID="profile-avatar"
          >
            <Text style={{ fontSize: 36, fontWeight: 'bold', color: colors.text.primary }}>{initial}</Text>
          </View>
          <Text style={{ fontSize: 14, color: colors.text.tertiary }}>{user?.email}</Text>
        </View>

        <View style={cardStyle}>
          <Text style={{ fontSize: 14, fontWeight: '600', color: colors.text.secondary, marginBottom: 8 }}>
            {t('app.displayName')}
          </Text>
          <Input
            value={displayName}
            onChangeText={setDisplayName}
            placeholder={t('app.howCoachAddressesYou')}
            autoCapitalize="words"
            testID="profile-display-name-input"
          />

          <Text style={{ fontSize: 14, fontWeight: '600', color: colors.text.secondary, marginTop: 20, marginBottom: 8 }}>
            {t('common.email')}
          </Text>
          {/* Read-only: the email identifies the account and is not editable here. */}
          <View
            style={{
              backgroundColor: colors.background.secondary,
              borderWidth: 1,
              borderColor: colors.border.subtle,
              borderRadius: 12,
              paddingHorizontal: 16,
              paddingVertical: 14,
            }}
            testID="profile-email-readonly"
          >
            <Text style={{ fontSize: 16, color: colors.text.tertiary }}>{user?.email}</Text>
          </View>
        </View>

        {/* Appearance and language sit with the profile on web too — they are
            how the athlete's own copy of the app reads, not a pane of their
            own. */}
        <View testID="profile-appearance-section">
          <Text style={{ fontSize: 18, fontWeight: '600', color: colors.text.primary, marginBottom: 12 }}>
            {t('settings.appearance')}
          </Text>
          <View style={{ ...cardStyle, padding: 0, overflow: 'hidden' }}>
            {(['system', 'dark', 'light'] as const).map((option, idx, arr) => {
              const isSelected = appearancePref === option;
              const label =
                option === 'system'
                  ? t('settings.appearanceSystem')
                  : option === 'dark'
                    ? t('settings.appearanceDark')
                    : t('settings.appearanceLight');
              const description =
                option === 'system'
                  ? t('settings.appearanceSystemHint')
                  : option === 'dark'
                    ? t('settings.appearanceDarkHint')
                    : t('settings.appearanceLightHint');
              const icon = option === 'system' ? 'smartphone' : option === 'dark' ? 'moon' : 'sun';
              return (
                <TouchableOpacity
                  key={option}
                  testID={`appearance-option-${option}`}
                  accessibilityRole="radio"
                  accessibilityState={{ selected: isSelected }}
                  onPress={() => { void setAppearancePref(option as AppearancePref); }}
                  style={[
                    { flexDirection: 'row', alignItems: 'center', paddingVertical: 14, paddingHorizontal: 16 },
                    idx < arr.length - 1
                      ? { borderBottomWidth: 1, borderBottomColor: colors.border.subtle }
                      : {},
                  ]}
                >
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
                    <Feather name={icon} size={20} color={colors.text.secondary} />
                  </View>
                  <View style={{ flex: 1 }}>
                    <Text style={{ fontSize: 16, color: colors.text.primary }}>{label}</Text>
                    <Text style={{ fontSize: 13, color: colors.text.tertiary, marginTop: 2 }}>{description}</Text>
                  </View>
                  <Feather
                    name={isSelected ? 'check-circle' : 'circle'}
                    size={20}
                    color={isSelected ? colors.pierre.violet : colors.text.tertiary}
                  />
                </TouchableOpacity>
              );
            })}
          </View>
        </View>

        {/* The switcher sets the chrome language AND `users.locale`, so the
            coach answers in the language the athlete reads the app in. */}
        <View testID="profile-language-section">
          <Text style={{ fontSize: 18, fontWeight: '600', color: colors.text.primary, marginBottom: 4 }}>
            {t('settings.language')}
          </Text>
          <Text style={{ fontSize: 13, color: colors.text.tertiary, marginBottom: 12 }}>
            {t('settings.languageDescription')}
          </Text>
          <LanguageSwitcher serverLocale={user?.locale} />
        </View>

        <TouchableOpacity
          onPress={() => { void handleSave(); }}
          disabled={!isDirty || isSaving}
          testID="profile-save-button"
          style={{
            backgroundColor: isDirty && !isSaving ? colors.tokens.primary : colors.background.tertiary,
            borderRadius: 12,
            paddingVertical: 16,
            alignItems: 'center',
          }}
        >
          {isSaving ? (
            <ActivityIndicator color={colors.tokens.onPrimary} />
          ) : (
            <Text
              style={{
                fontSize: 16,
                fontWeight: '600',
                color: isDirty ? colors.tokens.onPrimary : colors.text.tertiary,
              }}
            >
              {t('app.saveChanges')}
            </Text>
          )}
        </TouchableOpacity>
      </PaneScrollView>
    </SafeAreaView>
  );
}
