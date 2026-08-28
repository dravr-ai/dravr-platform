// ABOUTME: Profile screen — edit display name, view the account email
// ABOUTME: Mobile counterpart of the web Settings t('common.profile') tab, sharing its updateProfile call

import React, { useState } from 'react';
import {
  View,
  Text,
  ScrollView,
  TouchableOpacity,
  ActivityIndicator,
  Alert,
  type ViewStyle,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useRouter } from 'expo-router';
import { Feather } from '@expo/vector-icons';
import { spacing, useThemeColors } from '../../constants/theme';
import { Input } from '../../components/ui';
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

      <ScrollView contentContainerStyle={{ padding: spacing.md, gap: spacing.lg }}>
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
      </ScrollView>
    </SafeAreaView>
  );
}
