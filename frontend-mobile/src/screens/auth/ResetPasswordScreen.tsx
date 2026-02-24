// ABOUTME: Password reset screen for entering the 6-digit code and new password
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
import { colors, spacing, glassCard, buttonGlow, gradients } from '../../constants/theme';
import type { NativeStackNavigationProp } from '@react-navigation/native-stack';
import type { RouteProp } from '@react-navigation/native';

type AuthStackParamList = {
  Login: undefined;
  Register: undefined;
  ForgotPassword: undefined;
  ResetPassword: { email: string };
  PendingApproval: undefined;
};

interface ResetPasswordScreenProps {
  navigation: NativeStackNavigationProp<AuthStackParamList, 'ResetPassword'>;
  route: RouteProp<AuthStackParamList, 'ResetPassword'>;
}

export function ResetPasswordScreen({ navigation, route }: ResetPasswordScreenProps) {
  const { email } = route.params;
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
      newErrors.code = 'Reset code is required';
    } else if (!/^\d{6}$/.test(code)) {
      newErrors.code = 'Please enter a valid 6-digit code';
    }

    if (!newPassword) {
      newErrors.newPassword = 'New password is required';
    } else if (newPassword.length < 8) {
      newErrors.newPassword = 'Password must be at least 8 characters';
    }

    if (!confirmPassword) {
      newErrors.confirmPassword = 'Please confirm your password';
    } else if (newPassword !== confirmPassword) {
      newErrors.confirmPassword = 'Passwords do not match';
    }

    setErrors(newErrors);
    return Object.keys(newErrors).length === 0;
  };

  const handleSubmit = async () => {
    if (!validateForm()) return;

    setIsLoading(true);
    try {
      await authApi.resetPassword(code, newPassword);
      Alert.alert(
        'Password Reset',
        'Your password has been reset successfully. Please sign in.',
        [{ text: 'OK', onPress: () => navigation.navigate('Login') }],
      );
    } catch (error) {
      let message = 'Reset failed. Please try again.';
      if (error instanceof Error) {
        if (error.message.includes('404') || error.message.includes('not found')) {
          message = 'Invalid or expired code. Please request a new one.';
        } else {
          message = error.message;
        }
      }
      Alert.alert('Reset Failed', message);
    } finally {
      setIsLoading(false);
    }
  };

  const handleResendCode = () => {
    navigation.navigate('ForgotPassword');
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
    <SafeAreaView className="flex-1 bg-background-primary" testID="reset-password-screen">
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
                <View className="w-14 h-14 rounded-xl items-center justify-center mb-3 bg-success-500/20">
                  <Ionicons name="shield-checkmark-outline" size={28} color={colors.success[500]} />
                </View>
                <Text className="text-xl font-bold text-text-primary mb-1">
                  Enter Reset Code
                </Text>
                <Text className="text-sm text-text-secondary text-center leading-[20px]">
                  We sent a 6-digit code to {email}
                </Text>
              </View>

              {/* Form */}
              <View className="mb-4">
                <Input
                  label="Reset Code"
                  placeholder="Enter 6-digit code"
                  value={code}
                  onChangeText={(text) => setCode(text.replace(/\D/g, '').slice(0, 6))}
                  keyboardType="number-pad"
                  maxLength={6}
                  autoFocus
                  error={errors.code}
                  testID="reset-code-input"
                />

                <Input
                  label="New Password"
                  placeholder="Minimum 8 characters"
                  value={newPassword}
                  onChangeText={setNewPassword}
                  secureTextEntry
                  showPasswordToggle
                  error={errors.newPassword}
                  testID="new-password-input"
                />

                <Input
                  label="Confirm New Password"
                  placeholder="Re-enter your password"
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
                  title="Reset password"
                  onPress={handleSubmit}
                  loading={isLoading}
                  fullWidth
                  style={glowButtonStyle}
                  testID="reset-password-button"
                />
              </View>

              {/* Actions */}
              <View className="flex-row justify-between items-center pt-2">
                <TouchableOpacity onPress={handleResendCode}>
                  <Text className="text-sm text-text-tertiary">Resend code</Text>
                </TouchableOpacity>
                <TouchableOpacity onPress={() => navigation.navigate('Login')}>
                  <Text className="text-sm font-semibold text-primary-500">Back to sign in</Text>
                </TouchableOpacity>
              </View>
            </View>
          </View>
        </ScrollView>
    </SafeAreaView>
  );
}
