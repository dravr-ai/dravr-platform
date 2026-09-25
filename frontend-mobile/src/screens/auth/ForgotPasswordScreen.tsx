// ABOUTME: Self-service forgot password screen for requesting a reset code
// ABOUTME: Plain page on the app canvas — no card shell, a headline is enough

import React, { useState } from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  Alert,
  type ViewStyle,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { authApi } from '../../services/api';
import { Button, FormScrollView, Input } from '../../components/ui';
import { spacing } from '../../constants/theme';
import { useRouter } from 'expo-router';
import { useTranslation } from '@pierre/i18n';
import { describeApiError } from '@pierre/ui-logic';

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
      Alert.alert(
        t('common.error'),
        describeApiError(error, { t, fallbackKey: 'app.somethingWentWrongRetry' }),
      );
    } finally {
      setIsLoading(false);
    }
  };

  const submitButtonStyle: ViewStyle = {
    marginTop: spacing.md,
  };

  return (
    <SafeAreaView className="flex-1 bg-background-primary" testID="forgot-password-screen">
        <FormScrollView
          contentContainerStyle={{
            flexGrow: 1,
            justifyContent: 'center',
            paddingHorizontal: spacing.lg,
            paddingVertical: spacing.xl,
          }}
        >
          <View className="px-6 py-8">
            {/* Header */}
            <View className="items-center mb-6">
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
                style={submitButtonStyle}
                testID="send-code-button"
              />
            </View>

            {/* Back to Login */}
            <View className="flex-row justify-center items-center gap-1 pt-2">
              <TouchableOpacity onPress={() => router.replace('/(auth)/login')}>
                <Text className="text-sm font-semibold text-primary">{t('app.backToSignIn')}</Text>
              </TouchableOpacity>
            </View>
          </View>
        </FormScrollView>
    </SafeAreaView>
  );
}
