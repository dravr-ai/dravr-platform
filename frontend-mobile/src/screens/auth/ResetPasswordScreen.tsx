// ABOUTME: Password reset screen for entering the emailed reset code and new password
// ABOUTME: Plain page on the app canvas — no card shell, a headline is enough

import React, { useRef, useState } from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  Alert,
  type TextInput,
  type ViewStyle,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { authApi } from '../../services/api';
import { Button, FormScrollView, Input } from '../../components/ui';
import { spacing } from '../../constants/theme';
import { useRouter, useLocalSearchParams } from 'expo-router';
import { useTranslation } from '@pierre/i18n';

export function ResetPasswordScreen() {
  const { t } = useTranslation();
  const router = useRouter();
  const { email } = useLocalSearchParams<{ email: string }>();
  const [code, setCode] = useState('');
  const [newPassword, setNewPassword] = useState('');
  const [confirmPassword, setConfirmPassword] = useState('');
  const [isLoading, setIsLoading] = useState(false);
  const [errors, setErrors] = useState<{
    code?: string;
    newPassword?: string;
    confirmPassword?: string;
  }>({});

  const validateForm = () => {
    const newErrors: typeof errors = {};

    if (!code.trim()) {
      newErrors.code = t('validation.resetCodeRequired');
    } else if (!/^[A-Za-z0-9]+\.[A-Za-z0-9]+$/.test(code.trim())) {
      newErrors.code = t('app.enterResetCodeFromEmail');
    }

    if (!newPassword) {
      newErrors.newPassword = t('validation.newPasswordRequired');
    } else if (newPassword.length < 8) {
      newErrors.newPassword = t('app.passwordTooShort');
    }

    if (!confirmPassword) {
      newErrors.confirmPassword = t('validation.confirmPassword');
    } else if (newPassword !== confirmPassword) {
      newErrors.confirmPassword = t('app.passwordsDoNotMatch');
    }

    setErrors(newErrors);
    return Object.keys(newErrors).length === 0;
  };

  // Chains focus down the form: `returnKeyType` only relabels the return key,
  // so without refs to move the caret the athlete must aim at each next field
  // with the keyboard already covering it (carnet#353).
  const newPasswordRef = useRef<TextInput>(null);
  const confirmRef = useRef<TextInput>(null);

  const handleSubmit = async () => {
    if (!validateForm()) return;

    setIsLoading(true);
    try {
      await authApi.resetPassword(code.trim(), newPassword);
      Alert.alert(
        t('app.passwordResetTitle'),
        t('app.passwordResetBody'),
        [{ text: 'OK', onPress: () => router.replace('/(auth)/login') }],
      );
    } catch (error) {
      let message = t('app.resetFailedRetry');
      if (error instanceof Error) {
        if (error.message.includes('404') || error.message.includes('not found')) {
          message = t('app.codeInvalidOrExpired');
        } else {
          message = error.message;
        }
      }
      Alert.alert(t('app.resetFailed'), message);
    } finally {
      setIsLoading(false);
    }
  };

  const handleResendCode = () => {
    router.push('/(auth)/forgot-password');
  };

  const submitButtonStyle: ViewStyle = {
    marginTop: spacing.md,
  };

  return (
    <SafeAreaView className="flex-1 bg-background-primary" testID="reset-password-screen">
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
                {t('app.enterResetCode')}
              </Text>
              <Text className="text-sm text-text-secondary text-center leading-[20px]">
                {t('app.sentResetCodeTo', { email })}
              </Text>
            </View>

            {/* Form */}
            <View className="mb-4">
              <Input
                label={t('app.resetCode')}
                placeholder={t('app.pasteCodeFromEmail')}
                value={code}
                onChangeText={setCode}
                autoCapitalize="none"
                autoCorrect={false}
                autoFocus
                error={errors.code}
                returnKeyType="next"
                onSubmitEditing={() => newPasswordRef.current?.focus()}
                blurOnSubmit={false}
                testID="reset-code-input"
              />

              <Input
                ref={newPasswordRef}
                label={t('app.newPassword')}
                placeholder={t('app.minEightChars')}
                value={newPassword}
                onChangeText={setNewPassword}
                secureTextEntry
                showPasswordToggle
                error={errors.newPassword}
                returnKeyType="next"
                onSubmitEditing={() => confirmRef.current?.focus()}
                blurOnSubmit={false}
                testID="new-password-input"
              />

              <Input
                ref={confirmRef}
                label={t('app.confirmNewPassword')}
                placeholder={t('app.reenterPassword')}
                value={confirmPassword}
                onChangeText={setConfirmPassword}
                secureTextEntry
                showPasswordToggle
                returnKeyType="go"
                onSubmitEditing={handleSubmit}
                error={errors.confirmPassword}
                testID="confirm-password-input"
              />

              <Button
                title={t('app.resetPassword')}
                onPress={handleSubmit}
                loading={isLoading}
                fullWidth
                style={submitButtonStyle}
                testID="reset-password-button"
              />
            </View>

            {/* Actions */}
            <View className="flex-row justify-between items-center pt-2">
              <TouchableOpacity onPress={handleResendCode}>
                <Text className="text-sm font-semibold text-primary">{t('app.resendCode')}</Text>
              </TouchableOpacity>
              <TouchableOpacity onPress={() => router.replace('/(auth)/login')}>
                <Text className="text-sm font-semibold text-primary">{t('app.backToSignIn')}</Text>
              </TouchableOpacity>
            </View>
          </View>
        </FormScrollView>
    </SafeAreaView>
  );
}
