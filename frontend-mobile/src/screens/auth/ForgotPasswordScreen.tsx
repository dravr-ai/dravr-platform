// ABOUTME: Self-service forgot password screen for requesting a reset code
// ABOUTME: Professional dark theme UI with glassmorphism matching LoginScreen design

import React, { useState } from 'react';
import {
  View,
  Text,
  ScrollView,
  TouchableOpacity,
  Alert,
  type ViewStyle,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { LinearGradient } from 'expo-linear-gradient';
import { Ionicons } from '@expo/vector-icons';
import { authApi } from '../../services/api';
import { Button, Input } from '../../components/ui';
import { PRIMARY_PALETTE, spacing, glassCard, buttonGlow, gradients } from '../../constants/theme';
import { useRouter } from 'expo-router';
import { useTranslation } from '@pierre/i18n';

export function ForgotPasswordScreen() {
  const { t } = useTranslation();
  const router = useRouter();
  const [email, setEmail] = useState('');
  const [isLoading, setIsLoading] = useState(false);
  const [errors, setErrors] = useState<{ email?: string }>({});

  const validateForm = () => {
    const newErrors: { email?: string } = {};

    if (!email.trim()) {
      newErrors.email = t('validation.emailRequired');
    } else if (!/\S+@\S+\.\S+/.test(email)) {
      newErrors.email = t('validation.email');
    }

    setErrors(newErrors);
    return Object.keys(newErrors).length === 0;
  };

  const handleSubmit = async () => {
    if (!validateForm()) return;

    setIsLoading(true);
    try {
      await authApi.forgotPassword(email.trim());
      router.push({ pathname: '/(auth)/reset-password', params: { email: email.trim() } });
    } catch (error) {
      let message = t('app.somethingWentWrongRetry');
      if (error instanceof Error) {
        message = error.message;
      }
      Alert.alert(t('common.error'), message);
    } finally {
      setIsLoading(false);
    }
  };

  const cardStyle: ViewStyle = {
    ...glassCard,
    borderRadius: 16,
    overflow: 'hidden',
  };

  const glowButtonStyle: ViewStyle = {
    ...buttonGlow,
    marginTop: spacing.md,
  };

  return (
    <SafeAreaView className="flex-1 bg-background-primary" testID="forgot-password-screen">
        <ScrollView
          contentContainerStyle={{
            flexGrow: 1,
            justifyContent: 'center',
            paddingHorizontal: spacing.lg,
            paddingVertical: spacing.xl,
          }}
          keyboardShouldPersistTaps="handled"
          automaticallyAdjustKeyboardInsets
        >
          <View style={cardStyle}>
            <LinearGradient
              colors={gradients.violetCyan as [string, string]}
              start={{ x: 0, y: 0 }}
              end={{ x: 1, y: 0 }}
              style={{ height: 3, width: '100%' }}
            />

            <View className="px-6 py-8">
              {/* Header */}
              <View className="items-center mb-6">
                <View className="w-14 h-14 rounded-xl items-center justify-center mb-3 bg-primary-500/20">
                  <Ionicons name="lock-closed-outline" size={28} color={PRIMARY_PALETTE[500]} />
                </View>
                <Text className="text-xl font-bold text-text-primary mb-1">
                  {t('app.resetYourPassword')}
                </Text>
                <Text className="text-sm text-text-secondary text-center leading-[20px]">
                  {t('app.forgotPasswordBlurb')}
                </Text>
              </View>

              {/* Form */}
              <View className="mb-4">
                <Input
                  label={t('common.email')}
                  placeholder="you@example.com"
                  value={email}
                  onChangeText={setEmail}
                  keyboardType="email-address"
                  autoCapitalize="none"
                  autoCorrect={false}
                  returnKeyType="go"
                  onSubmitEditing={handleSubmit}
                  error={errors.email}
                  testID="forgot-email-input"
                />

                <Button
                  title={t('app.sendResetCode')}
                  onPress={handleSubmit}
                  loading={isLoading}
                  fullWidth
                  style={glowButtonStyle}
                  testID="send-code-button"
                />
              </View>

              {/* Back to Login */}
              <View className="flex-row justify-center items-center gap-1 pt-2">
                <TouchableOpacity onPress={() => router.replace('/(auth)/login')}>
                  <Text className="text-sm font-semibold text-primary-500">{t('app.backToSignIn')}</Text>
                </TouchableOpacity>
              </View>
            </View>
          </View>
        </ScrollView>
    </SafeAreaView>
  );
}
