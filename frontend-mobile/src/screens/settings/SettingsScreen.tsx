// ABOUTME: Profile & Settings screen with Stitch UX design
// ABOUTME: Shows profile header, stats, connected services, and settings sections

import React, { useState, useEffect, useCallback, useMemo } from 'react';
import {
  View,
  Text,
  ScrollView,
  TouchableOpacity,
  Alert,
  Modal,
  ActivityIndicator,
  type ViewStyle,
} from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { useFocusEffect } from 'expo-router';
import { useRouter } from 'expo-router';
import { LinearGradient } from 'expo-linear-gradient';
import { Feather } from '@expo/vector-icons';
import { spacing, borderRadius, useThemeColors, useTheme } from '../../constants/theme';
import type { AppearancePref } from '../../hooks/useAppearancePref';
import { Input } from '../../components/ui';
import { useAuth } from '../../contexts/AuthContext';
import { userApi, oauthApi } from '../../services/api';
import { useUsageStatus, type LimitCheckResult } from '../chat/useUsageStatus';
import type { McpToken, ExtendedProviderStatus } from '../../types';
// Settings row style
const settingsRowStyle: ViewStyle = {
  flexDirection: 'row',
  alignItems: 'center',
  paddingVertical: 14,
  paddingHorizontal: 16,
};

/** Format large numbers compactly (e.g. 145000 -> "145.0K") */
function formatCompactNumber(value: number): string {
  if (value >= 1_000_000) {
    return `${(value / 1_000_000).toFixed(1)}M`;
  }
  if (value >= 1_000) {
    return `${(value / 1_000).toFixed(1)}K`;
  }
  return value.toLocaleString();
}

/** Return hex color based on usage percentage: green < 70%, amber 70-90%, red > 90% */
function getUsageBarColor(
  current: number,
  limit: number,
  palette: { activity: string; nutrition: string; red: string },
): string {
  if (limit <= 0) return palette.activity;
  const pct = (current / limit) * 100;
  if (pct > 90) return palette.red;
  if (pct > 70) return palette.nutrition;
  return palette.activity;
}

/** Format ISO 8601 reset time in user's local timezone */
function formatResetTime(isoString: string): string {
  try {
    const date = new Date(isoString);
    return new Intl.DateTimeFormat(undefined, {
      hour: 'numeric',
      minute: '2-digit',
      timeZoneName: 'short',
    }).format(date);
  } catch {
    return 'midnight UTC';
  }
}

export function SettingsScreen() {
  const router = useRouter();
  const { user, logout, isAuthenticated } = useAuth();
  const insets = useSafeAreaInsets();
  const colors = useThemeColors();
  const { pref: appearancePref, setPref: setAppearancePref } = useTheme();

  // Glassmorphism card style — derived per render so it tracks the active scheme.
  const glassCardStyle: ViewStyle = {
    backgroundColor: colors.background.tertiary,
    borderWidth: 1,
    borderColor: colors.border.default,
    borderRadius: 16,
  };
  const [tokens, setTokens] = useState<McpToken[]>([]);
  const [showCreateToken, setShowCreateToken] = useState(false);
  const [showChangePassword, setShowChangePassword] = useState(false);
  const [newTokenName, setNewTokenName] = useState('');
  const [isCreatingToken, setIsCreatingToken] = useState(false);
  const [newToken, setNewToken] = useState<string | null>(null);
  const [connectedProviders, setConnectedProviders] = useState<ExtendedProviderStatus[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);

  // Password change state
  const [currentPassword, setCurrentPassword] = useState('');
  const [newPassword, setNewPassword] = useState('');
  const [confirmPassword, setConfirmPassword] = useState('');
  const [isChangingPassword, setIsChangingPassword] = useState(false);

  useEffect(() => {
    if (isAuthenticated) {
      loadTokens();
    }
  }, [isAuthenticated]);

  // Reload provider status when screen comes into focus (e.g., after OAuth connection)
  useFocusEffect(
    useCallback(() => {
      if (isAuthenticated) {
        loadProviderStatus();
      }
    }, [isAuthenticated])
  );

  const loadTokens = async () => {
    try {
      setLoadError(null);
      const response = await userApi.getMcpTokens();
      const tokenList = response.tokens || [];
      const seen = new Set<string>();
      const deduplicated = tokenList.filter((t: { id: string; is_revoked: boolean }) => {
        if (t.is_revoked || seen.has(t.id)) return false;
        seen.add(t.id);
        return true;
      });
      setTokens(deduplicated);
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : 'Failed to load tokens';
      setLoadError(errorMessage);
      console.error('Failed to load tokens:', err);
      setTokens([]);
    }
  };

  const loadProviderStatus = async () => {
    try {
      // Use getProvidersStatus() to include non-OAuth providers like synthetic
      const response = await oauthApi.getProvidersStatus();
      setConnectedProviders(response.providers || []);
    } catch (err) {
      console.error('Failed to load provider status:', err);
    }
  };

  const handleCreateToken = async () => {
    if (!newTokenName.trim()) {
      Alert.alert('Error', 'Please enter a token name');
      return;
    }

    try {
      setIsCreatingToken(true);
      const token = await userApi.createMcpToken({
        name: newTokenName.trim(),
        expires_in_days: 365,
      });
      setNewToken(token.token_value || 'Token created successfully');
      await loadTokens();
      setNewTokenName('');
    } catch {
      Alert.alert('Error', 'Failed to create token');
    } finally {
      setIsCreatingToken(false);
    }
  };

  const handleChangePassword = async () => {
    if (!currentPassword || !newPassword || !confirmPassword) {
      Alert.alert('Error', 'Please fill in all fields');
      return;
    }

    if (newPassword !== confirmPassword) {
      Alert.alert('Error', 'New passwords do not match');
      return;
    }

    if (newPassword.length < 8) {
      Alert.alert('Error', 'Password must be at least 8 characters');
      return;
    }

    try {
      setIsChangingPassword(true);
      await userApi.changePassword(currentPassword, newPassword);
      Alert.alert('Success', 'Password changed successfully');
      setShowChangePassword(false);
      setCurrentPassword('');
      setNewPassword('');
      setConfirmPassword('');
    } catch {
      Alert.alert('Error', 'Failed to change password. Please check your current password.');
    } finally {
      setIsChangingPassword(false);
    }
  };

  const handleLogout = () => {
    Alert.alert(
      'Sign Out',
      'Are you sure you want to sign out?',
      [
        { text: 'Cancel', style: 'cancel' },
        {
          text: 'Sign Out',
          style: 'destructive',
          onPress: logout,
        },
      ]
    );
  };

  // Usage quota status
  const { data: usageData, isLoading: usageLoading } = useUsageStatus();

  const usageBars = useMemo(() => {
    if (!usageData) return [];
    return [
      { label: 'Daily Messages', counter: usageData.daily.messages, compact: false },
      { label: 'Daily Tokens', counter: usageData.daily.tokens, compact: true },
      { label: 'Weekly Messages', counter: usageData.weekly.messages, compact: false },
    ] as { label: string; counter: LimitCheckResult; compact: boolean }[];
  }, [usageData]);

  const displayName = user?.display_name || user?.email?.split('@')[0] || 'Athlete';

  return (
    <View style={{ flex: 1, backgroundColor: colors.background.primary }} testID="settings-screen">
      <ScrollView
        style={{ flex: 1 }}
        contentContainerStyle={{
          paddingTop: insets.top + spacing.sm,
          paddingBottom: 100,
          paddingHorizontal: spacing.md,
        }}
        showsVerticalScrollIndicator={false}
      >
        {/* Profile Header with gradient-bordered avatar */}
        <View style={{ alignItems: 'center', paddingHorizontal: 16, paddingVertical: 24 }} testID="settings-profile-section">
          {/* Load Error Display */}
          {loadError && (
            <View className="w-full mb-4 p-3 bg-error/10 border border-error/30 rounded-lg flex-row items-center justify-between">
              <Text className="flex-1 text-error text-sm mr-3">{loadError}</Text>
              <TouchableOpacity
                className="px-3 py-1.5 bg-error/20 rounded-md"
                onPress={() => {
                  setLoadError(null);
                  loadTokens();
                }}
              >
                <Text className="text-error text-sm font-semibold">Retry</Text>
              </TouchableOpacity>
            </View>
          )}
          {/* Gradient-bordered Avatar */}
          <LinearGradient
            colors={[colors.pierre.violet, colors.pierre.cyan]}
            start={{ x: 0, y: 0 }}
            end={{ x: 1, y: 1 }}
            style={{
              width: 112,
              height: 112,
              borderRadius: 56,
              alignItems: 'center',
              justifyContent: 'center',
              marginBottom: 16,
              padding: 4,
            }}
          >
            <View style={{
              width: '100%',
              height: '100%',
              borderRadius: 56,
              backgroundColor: colors.background.primary,
              alignItems: 'center',
              justifyContent: 'center',
            }}>
              <Text style={{ fontSize: 36, fontWeight: 'bold', color: colors.text.primary }}>
                {displayName[0]?.toUpperCase() || 'U'}
              </Text>
            </View>
          </LinearGradient>

          <Text style={{ fontSize: 24, fontWeight: 'bold', color: colors.text.primary, marginBottom: 4 }}>{displayName}</Text>
          <Text style={{ fontSize: 16, color: colors.text.tertiary, marginBottom: 16 }}>{user?.email}</Text>

          {/* Edit Profile Button with violet glow */}
          <TouchableOpacity
            style={{
              paddingHorizontal: 24,
              paddingVertical: 10,
              borderRadius: 9999,
              backgroundColor: colors.pierre.violet,
              shadowColor: colors.pierre.violet,
              shadowOffset: { width: 0, height: 0 },
              shadowOpacity: 0.4,
              shadowRadius: 12,
              elevation: 6,
            }}
          >
            <Text style={{ fontSize: 14, fontWeight: '600', color: colors.tokens.onPrimary }}>Edit Profile</Text>
          </TouchableOpacity>
        </View>

        {/* Data Providers Section - navigates to Connections screen */}
        <View style={{ paddingHorizontal: 16, marginBottom: 24 }} testID="settings-data-section">
          <Text style={{ fontSize: 18, fontWeight: '600', color: colors.text.primary, marginBottom: 12 }}>Data</Text>
          <View style={glassCardStyle}>
            <TouchableOpacity
              style={settingsRowStyle}
              onPress={() => router.push('/(app)/(tabs)/(settings)/connections')}
              testID="settings-data-providers-button"
            >
              <View style={{ width: 40, height: 40, borderRadius: 12, backgroundColor: colors.background.secondary, alignItems: 'center', justifyContent: 'center', marginRight: 12 }}>
                <Feather name="link" size={20} color={colors.text.secondary} />
              </View>
              <View style={{ flex: 1 }}>
                <Text style={{ fontSize: 16, color: colors.text.primary }}>Data Providers</Text>
                <Text style={{ fontSize: 14, color: colors.text.tertiary }}>
                  {connectedProviders.filter(p => p.connected).length} connected
                </Text>
              </View>
              <Feather name="chevron-right" size={20} color={colors.text.tertiary} />
            </TouchableOpacity>
          </View>
        </View>

        {/* Coaching Style Section */}
        <View style={{ paddingHorizontal: 16, marginBottom: 24 }} testID="settings-coaching-section">
          <Text style={{ fontSize: 18, fontWeight: '600', color: colors.text.primary, marginBottom: 12 }}>Coaching</Text>
          <View style={glassCardStyle}>
            <TouchableOpacity
              style={settingsRowStyle}
              onPress={() => router.push('/(app)/(tabs)/(settings)/coaching-style')}
              testID="settings-coaching-style-button"
            >
              <View style={{ width: 40, height: 40, borderRadius: 12, backgroundColor: colors.background.secondary, alignItems: 'center', justifyContent: 'center', marginRight: 12 }}>
                <Feather name="message-square" size={20} color={colors.text.secondary} />
              </View>
              <View style={{ flex: 1 }}>
                <Text style={{ fontSize: 16, color: colors.text.primary }}>Coaching Style</Text>
                <Text style={{ fontSize: 14, color: colors.text.tertiary, textTransform: 'capitalize' }}>
                  {(user?.coaching_persona ?? 'casual').replace('_', '-')}
                </Text>
              </View>
              <Feather name="chevron-right" size={20} color={colors.text.tertiary} />
            </TouchableOpacity>
          </View>
        </View>

        {/* Account Settings Section */}
        <View style={{ paddingHorizontal: 16, marginBottom: 24 }} testID="settings-account-section">
          <Text style={{ fontSize: 18, fontWeight: '600', color: colors.text.primary, marginBottom: 12 }}>Account</Text>
          <View style={glassCardStyle}>
            <TouchableOpacity style={[settingsRowStyle, { borderBottomWidth: 1, borderBottomColor: colors.border.subtle }]} testID="settings-personal-info-button">
              <View style={{ width: 40, height: 40, borderRadius: 12, backgroundColor: colors.background.secondary, alignItems: 'center', justifyContent: 'center', marginRight: 12 }}>
                <Feather name="user" size={20} color={colors.text.secondary} />
              </View>
              <Text style={{ flex: 1, fontSize: 16, color: colors.text.primary }}>Personal Information</Text>
              <Feather name="chevron-right" size={20} color={colors.text.tertiary} />
            </TouchableOpacity>

            <TouchableOpacity
              style={[settingsRowStyle, { borderBottomWidth: 1, borderBottomColor: colors.border.subtle }]}
              onPress={() => setShowChangePassword(true)}
              testID="settings-change-password-button"
            >
              <View style={{ width: 40, height: 40, borderRadius: 12, backgroundColor: colors.background.secondary, alignItems: 'center', justifyContent: 'center', marginRight: 12 }}>
                <Feather name="lock" size={20} color={colors.text.secondary} />
              </View>
              <Text style={{ flex: 1, fontSize: 16, color: colors.text.primary }}>Change Password</Text>
              <Feather name="chevron-right" size={20} color={colors.text.tertiary} />
            </TouchableOpacity>

            <TouchableOpacity style={settingsRowStyle} onPress={() => setShowCreateToken(true)} testID="settings-mcp-tokens-button">
              <View style={{ width: 40, height: 40, borderRadius: 12, backgroundColor: colors.background.secondary, alignItems: 'center', justifyContent: 'center', marginRight: 12 }}>
                <Feather name="key" size={20} color={colors.text.secondary} />
              </View>
              <View style={{ flex: 1 }}>
                <Text style={{ fontSize: 16, color: colors.text.primary }}>MCP Tokens</Text>
                <Text style={{ fontSize: 14, color: colors.text.tertiary }}>{tokens.length} active</Text>
              </View>
              <Feather name="chevron-right" size={20} color={colors.text.tertiary} />
            </TouchableOpacity>
          </View>
        </View>

        {/* Appearance Section — System / Light / Dark */}
        <View style={{ paddingHorizontal: 16, marginBottom: 24 }} testID="settings-appearance-section">
          <Text style={{ fontSize: 18, fontWeight: '600', color: colors.text.primary, marginBottom: 12 }}>Appearance</Text>
          <View style={glassCardStyle}>
            {(['system', 'dark', 'light'] as const).map((option, idx, arr) => {
              const isSelected = appearancePref === option;
              const label = option === 'system' ? 'System' : option === 'dark' ? 'Dark' : 'Light';
              const description =
                option === 'system'
                  ? 'Follow device setting'
                  : option === 'dark'
                    ? 'Boreal ink — recommended'
                    : 'Boreal paper';
              const icon = option === 'system' ? 'smartphone' : option === 'dark' ? 'moon' : 'sun';
              const isLast = idx === arr.length - 1;
              return (
                <TouchableOpacity
                  key={option}
                  testID={`appearance-option-${option}`}
                  accessibilityRole="radio"
                  accessibilityState={{ selected: isSelected }}
                  onPress={() => {
                    void setAppearancePref(option as AppearancePref);
                  }}
                  style={[
                    settingsRowStyle,
                    !isLast && { borderBottomWidth: 1, borderBottomColor: colors.border.subtle },
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

        {/* Usage Section */}
        <View style={{ paddingHorizontal: 16, marginBottom: 24 }} testID="settings-usage-section">
          <Text style={{ fontSize: 18, fontWeight: '600', color: colors.text.primary, marginBottom: 12 }}>Usage</Text>
          <View style={glassCardStyle}>
            {usageLoading ? (
              <View style={{ paddingVertical: 24, alignItems: 'center' }}>
                <ActivityIndicator size="small" color={colors.pierre.violet} />
              </View>
            ) : !usageData ? (
              <View style={{ paddingVertical: 24, alignItems: 'center' }}>
                <Text style={{ fontSize: 14, color: colors.text.tertiary }}>Usage data unavailable</Text>
              </View>
            ) : (
              <View style={{ padding: 16 }}>
                {/* Progress bars */}
                {usageBars.map(({ label, counter, compact }) => {
                  const pct = counter.limit > 0 ? Math.min((counter.current / counter.limit) * 100, 100) : 0;
                  return (
                    <View key={label} style={{ marginBottom: 16 }}>
                      <View style={{ flexDirection: 'row', justifyContent: 'space-between', marginBottom: 6 }}>
                        <Text style={{ fontSize: 14, fontWeight: '500', color: colors.text.secondary }}>{label}</Text>
                        <Text style={{ fontSize: 14, color: colors.text.tertiary }}>
                          {compact ? formatCompactNumber(counter.current) : counter.current.toLocaleString()}
                          {' / '}
                          {compact ? formatCompactNumber(counter.limit) : counter.limit.toLocaleString()}
                        </Text>
                      </View>
                      <View style={{ height: 8, backgroundColor: colors.background.tertiary, borderRadius: 4, overflow: 'hidden' }}>
                        <View
                          style={{
                            height: '100%',
                            width: `${pct}%`,
                            backgroundColor: getUsageBarColor(counter.current, counter.limit, {
                              activity: colors.pierre.activity,
                              nutrition: colors.pierre.nutrition,
                              red: colors.pierre.red,
                            }),
                            borderRadius: 4,
                          }}
                        />
                      </View>
                    </View>
                  );
                })}

                {/* Reset time */}
                <Text style={{ fontSize: 12, color: colors.text.tertiary, marginBottom: 16 }}>
                  Daily limits reset at {formatResetTime(usageData.daily.messages.resets_at)}
                </Text>

                {/* Resource counts */}
                <View style={{ borderTopWidth: 1, borderTopColor: colors.border.default, paddingTop: 16 }}>
                  <View style={{ flexDirection: 'row', gap: 12 }}>
                    <View style={{ flex: 1, backgroundColor: colors.background.tertiary, borderRadius: 8, padding: 12 }}>
                      <Text style={{ fontSize: 12, color: colors.text.tertiary, marginBottom: 4 }}>Coaches</Text>
                      <Text style={{ fontSize: 14, fontWeight: '500', color: colors.text.primary }}>
                        {usageData.resources.coaches} / {usageData.resources.max_coaches}
                      </Text>
                    </View>
                    <View style={{ flex: 1, backgroundColor: colors.background.tertiary, borderRadius: 8, padding: 12 }}>
                      <Text style={{ fontSize: 12, color: colors.text.tertiary, marginBottom: 4 }}>Conversations</Text>
                      <Text style={{ fontSize: 14, fontWeight: '500', color: colors.text.primary }}>
                        {usageData.resources.conversations} / {usageData.resources.max_conversations}
                      </Text>
                    </View>
                  </View>
                </View>
              </View>
            )}
          </View>
        </View>

        {/* Privacy Section */}
        <View style={{ paddingHorizontal: 16, marginBottom: 24 }}>
          <Text style={{ fontSize: 18, fontWeight: '600', color: colors.text.primary, marginBottom: 12 }}>Privacy</Text>
          <View style={glassCardStyle}>
            <TouchableOpacity style={settingsRowStyle}>
              <View style={{ width: 40, height: 40, borderRadius: 12, backgroundColor: colors.background.secondary, alignItems: 'center', justifyContent: 'center', marginRight: 12 }}>
                <Feather name="shield" size={20} color={colors.text.secondary} />
              </View>
              <Text style={{ flex: 1, fontSize: 16, color: colors.text.primary }}>Privacy Settings</Text>
              <Feather name="chevron-right" size={20} color={colors.text.tertiary} />
            </TouchableOpacity>
          </View>
        </View>

        {/* About Section */}
        <View style={{ paddingHorizontal: 16, marginBottom: 24 }}>
          <Text style={{ fontSize: 18, fontWeight: '600', color: colors.text.primary, marginBottom: 12 }}>About</Text>
          <View style={glassCardStyle}>
            <TouchableOpacity style={[settingsRowStyle, { borderBottomWidth: 1, borderBottomColor: colors.border.subtle }]}>
              <View style={{ width: 40, height: 40, borderRadius: 12, backgroundColor: colors.background.secondary, alignItems: 'center', justifyContent: 'center', marginRight: 12 }}>
                <Feather name="info" size={20} color={colors.text.secondary} />
              </View>
              <View style={{ flex: 1 }}>
                <Text style={{ fontSize: 16, color: colors.text.primary }}>Version</Text>
                <Text style={{ fontSize: 14, color: colors.text.tertiary }}>1.0.0</Text>
              </View>
            </TouchableOpacity>

            <TouchableOpacity style={[settingsRowStyle, { borderBottomWidth: 1, borderBottomColor: colors.border.subtle }]}>
              <View style={{ width: 40, height: 40, borderRadius: 12, backgroundColor: colors.background.secondary, alignItems: 'center', justifyContent: 'center', marginRight: 12 }}>
                <Feather name="help-circle" size={20} color={colors.text.secondary} />
              </View>
              <Text style={{ flex: 1, fontSize: 16, color: colors.text.primary }}>Help Center</Text>
              <Feather name="chevron-right" size={20} color={colors.text.tertiary} />
            </TouchableOpacity>

            <TouchableOpacity style={settingsRowStyle}>
              <View style={{ width: 40, height: 40, borderRadius: 12, backgroundColor: colors.background.secondary, alignItems: 'center', justifyContent: 'center', marginRight: 12 }}>
                <Feather name="file-text" size={20} color={colors.text.secondary} />
              </View>
              <Text style={{ flex: 1, fontSize: 16, color: colors.text.primary }}>Terms & Privacy</Text>
              <Feather name="chevron-right" size={20} color={colors.text.tertiary} />
            </TouchableOpacity>
          </View>
        </View>

        {/* Log Out Button - soft red */}
        <View style={{ paddingHorizontal: 16, marginBottom: 24 }}>
          <TouchableOpacity
            style={[glassCardStyle, { borderColor: 'rgba(255, 107, 107, 0.3)', paddingVertical: 16, alignItems: 'center' }]}
            onPress={handleLogout}
            testID="settings-logout-button"
          >
            <Text style={{ fontSize: 16, fontWeight: '600', color: colors.pierre.red }}>Log Out</Text>
          </TouchableOpacity>
        </View>
      </ScrollView>

      {/* Create Token Modal */}
      <Modal
        visible={showCreateToken}
        animationType="slide"
        transparent
        onRequestClose={() => setShowCreateToken(false)}
      >
        <View
          className="flex-1 bg-black/70 justify-center"
          style={{ paddingHorizontal: spacing.lg }}
        >
          <View
            className="bg-surface-container-low p-5"
            style={{ borderRadius: borderRadius.xl }}
          >
            <Text className="text-xl font-semibold text-on-surface mb-5 text-center">
              {newToken ? 'Token Created' : 'Create MCP Token'}
            </Text>

            {newToken ? (
              <>
                <Text className="text-sm text-amber-500 text-center mb-3">
                  Copy this token now. You won't be able to see it again!
                </Text>
                <View className="bg-surface rounded-lg p-3 mb-5">
                  <Text className="text-sm text-on-surface font-mono" selectable>
                    {newToken}
                  </Text>
                </View>
                <TouchableOpacity
                  className="py-3 rounded-full items-center"
                  style={{ backgroundColor: colors.pierre.violet }}
                  onPress={() => {
                    setShowCreateToken(false);
                    setNewToken(null);
                  }}
                >
                  <Text className="text-base font-semibold" style={{ color: colors.tokens.onPrimary }}>Done</Text>
                </TouchableOpacity>
              </>
            ) : (
              <>
                <Input
                  label="Token Name"
                  placeholder="e.g., Claude Desktop"
                  value={newTokenName}
                  onChangeText={setNewTokenName}
                />
                <View className="flex-row gap-3 mt-4">
                  <TouchableOpacity
                    className="flex-1 py-3 rounded-full items-center"
                    style={{ backgroundColor: colors.background.tertiary }}
                    onPress={() => setShowCreateToken(false)}
                  >
                    <Text className="text-base font-semibold text-on-surface">Cancel</Text>
                  </TouchableOpacity>
                  <TouchableOpacity
                    className="flex-1 py-3 rounded-full items-center"
                    style={{ backgroundColor: colors.pierre.violet }}
                    onPress={handleCreateToken}
                    disabled={isCreatingToken}
                  >
                    {isCreatingToken ? (
                      <ActivityIndicator size="small" color={colors.tokens.onPrimary} />
                    ) : (
                      <Text className="text-base font-semibold" style={{ color: colors.tokens.onPrimary }}>Create</Text>
                    )}
                  </TouchableOpacity>
                </View>
              </>
            )}
          </View>
        </View>
      </Modal>

      {/* Change Password Modal */}
      <Modal
        visible={showChangePassword}
        animationType="slide"
        transparent
        onRequestClose={() => setShowChangePassword(false)}
      >
        <View
          className="flex-1 bg-black/70 justify-center"
          style={{ paddingHorizontal: spacing.lg }}
        >
          <View
            className="bg-surface-container-low p-5"
            style={{ borderRadius: borderRadius.xl }}
          >
            <Text className="text-xl font-semibold text-on-surface mb-5 text-center">
              Change Password
            </Text>

            <Input
              label="Current Password"
              value={currentPassword}
              onChangeText={setCurrentPassword}
              secureTextEntry
              showPasswordToggle
            />
            <Input
              label="New Password"
              value={newPassword}
              onChangeText={setNewPassword}
              secureTextEntry
              showPasswordToggle
            />
            <Input
              label="Confirm New Password"
              value={confirmPassword}
              onChangeText={setConfirmPassword}
              secureTextEntry
              showPasswordToggle
            />

            <View className="flex-row gap-3 mt-4">
              <TouchableOpacity
                className="flex-1 py-3 rounded-full items-center"
                style={{ backgroundColor: colors.background.tertiary }}
                onPress={() => setShowChangePassword(false)}
              >
                <Text className="text-base font-semibold text-on-surface">Cancel</Text>
              </TouchableOpacity>
              <TouchableOpacity
                className="flex-1 py-3 rounded-full items-center"
                style={{ backgroundColor: colors.pierre.violet }}
                onPress={handleChangePassword}
                disabled={isChangingPassword}
              >
                {isChangingPassword ? (
                  <ActivityIndicator size="small" color={colors.tokens.onPrimary} />
                ) : (
                  <Text className="text-base font-semibold" style={{ color: colors.tokens.onPrimary }}>Change</Text>
                )}
              </TouchableOpacity>
            </View>
          </View>
        </View>
      </Modal>
    </View>
  );
}
