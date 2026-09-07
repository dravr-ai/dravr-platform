// ABOUTME: Registration screen for new user signup
// ABOUTME: Resting card on the app canvas — its fill and hairline follow the athlete's colour scheme

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
import { useAsyncAction } from '@pierre/ui-logic';
import { useAuth } from '../../contexts/AuthContext';
import { Button, Input } from '../../components/ui';
import { BrandLockup } from '../../components/ui/BrandLockup';
import { spacing, useCardStyle, buttonGlow } from '../../constants/theme';
import { useRouter } from 'expo-router';
import { useTranslation } from '@pierre/i18n';

export function RegisterScreen() {
  const { t } = useTranslation();
  const router = useRouter();
  const { register } = useAuth();
  const [displayName, setDisplayName] = useState('');
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [confirmPassword, setConfirmPassword] = useState('');
  const [errors, setErrors] = useState<{
    displayName?: string;
    email?: string;
    password?: string;
    confirmPassword?: string;
  }>({});

  const validateForm = () => {
    const newErrors: typeof errors = {};

    if (!displayName.trim()) {
      newErrors.displayName = t('validation.displayNameRequired');
    }

    if (!email.trim()) {
      newErrors.email = t('validation.emailRequired');
    } else if (!/\S+@\S+\.\S+/.test(email)) {
      newErrors.email = t('validation.email');
    }

    if (!password) {
      newErrors.password = t('validation.passwordRequired');
    } else if (password.length < 8) {
      newErrors.password = t('app.passwordTooShort');
    }

    if (!confirmPassword) {
      newErrors.confirmPassword = t('validation.confirmPassword');
    } else if (password !== confirmPassword) {
      newErrors.confirmPassword = t('app.passwordsDoNotMatch');
    }

    setErrors(newErrors);
    return Object.keys(newErrors).length === 0;
  };

  // Delegate registration loading lifecycle to @pierre/ui-logic
  const registerAction = useAsyncAction({
    action: () => register(email.trim(), password, displayName.trim()),
    onSuccess: () => {
      router.replace('/(auth)/pending-approval');
    },
    onError: (error: unknown) => {
      const message = error instanceof Error ? error.message : t('app.registrationFailedLower');
      Alert.alert(t('app.registrationFailedTitle'), message);
    },
    successResetDelay: 0,
    errorResetDelay: 0,
  });

  const handleRegister = () => {
    if (!validateForm()) return;
    registerAction.execute();
  };

  const cardStyle: ViewStyle = {
    ...useCardStyle(),
    borderRadius: 16,
    overflow: 'hidden',
  };

  // Button with glow effect
  const glowButtonStyle: ViewStyle = {
    ...buttonGlow,
    marginTop: spacing.md,
  };

  return (
    <SafeAreaView className="flex-1 bg-background-primary">
        <ScrollView
          contentContainerStyle={{ flexGrow: 1, justifyContent: 'center', paddingHorizontal: spacing.lg, paddingVertical: spacing.xl }}
          keyboardShouldPersistTaps="handled"
          automaticallyAdjustKeyboardInsets
        >
          {/* Card container */}
          <View style={cardStyle}>
            <View className="px-6 py-8">
              {/* Header */}
              <View className="items-center mb-6">
                {/* The product's own mark. This was a gradient square holding a
                    bold "P" — the Pierre-era monogram, on a fixed fill, with
                    body ink over it. Three things wrong at once: the wrong
                    brand, a colour that ignored the athlete's scheme, and an
                    ink not bound to its ground. `BrandLockup` is the one
                    lockup both clients draw. */}
                <View className="mb-3">
                  <BrandLockup size={36} testID="register-lockup" />
                </View>
                <Text className="text-xl font-bold text-text-primary mb-1">{t('app.createAccount')}</Text>
                <Text className="text-sm text-text-secondary text-center leading-[20px]">
                  {t('app.joinDravrBlurb')}
                </Text>
              </View>

              {/* Registration Form */}
              <View className="mb-4">
                <Input
                  label={t('app.displayName')}
                  placeholder={t('app.howShouldWeCallYou')}
                  value={displayName}
                  onChangeText={setDisplayName}
                  autoCapitalize="words"
                  error={errors.displayName}
                />

                <Input
                  label={t('common.email')}
                  placeholder="you@example.com"
                  value={email}
                  onChangeText={setEmail}
                  keyboardType="email-address"
                  autoCapitalize="none"
                  autoCorrect={false}
                  error={errors.email}
                />

                <Input
                  label={t('common.password')}
                  placeholder={t('app.minEightChars')}
                  value={password}
                  onChangeText={setPassword}
                  secureTextEntry
                  showPasswordToggle
                  error={errors.password}
                />

                <Input
                  label={t('app.confirmPassword')}
                  placeholder={t('app.reenterPassword')}
                  value={confirmPassword}
                  onChangeText={setConfirmPassword}
                  secureTextEntry
                  showPasswordToggle
                  error={errors.confirmPassword}
                />

                <Button
                  title={t('app.createAccount')}
                  onPress={handleRegister}
                  loading={registerAction.isLoading}
                  fullWidth
                  style={glowButtonStyle}
                />
              </View>

              {/* Login Link */}
              <View className="flex-row justify-center items-center gap-1 pt-2">
                <Text className="text-sm text-text-secondary">{t('app.alreadyHaveAccount')}</Text>
                <TouchableOpacity onPress={() => router.replace('/(auth)/login')}>
                  <Text className="text-sm font-semibold text-primary-500">{t('common.login')}</Text>
                </TouchableOpacity>
              </View>
            </View>
          </View>
        </ScrollView>
    </SafeAreaView>
  );
}
