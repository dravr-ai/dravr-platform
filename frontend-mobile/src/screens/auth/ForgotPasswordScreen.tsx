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
import { colors, spacing, glassCard, buttonGlow, gradients } from '../../constants/theme';
import type { NativeStackNavigationProp } from '@react-navigation/native-stack';

type AuthStackParamList = {
  Login: undefined;
  Register: undefined;
  ForgotPassword: undefined;
  ResetPassword: { email: string };
  PendingApproval: undefined;
};

interface ForgotPasswordScreenProps {
  navigation: NativeStackNavigationProp<AuthStackParamList, 'ForgotPassword'>;
}

export function ForgotPasswordScreen({ navigation }: ForgotPasswordScreenProps) {
  const [email, setEmail] = useState('');
  const [isLoading, setIsLoading] = useState(false);
  const [errors, setErrors] = useState<{ email?: string }>({});

  const validateForm = () => {
    const newErrors: { email?: string } = {};

    if (!email.trim()) {
      newErrors.email = 'Email is required';
    } else if (!/\S+@\S+\.\S+/.test(email)) {
      newErrors.email = 'Please enter a valid email';
    }

    setErrors(newErrors);
    return Object.keys(newErrors).length === 0;
  };

  const handleSubmit = async () => {
    if (!validateForm()) return;

    setIsLoading(true);
    try {
      await authApi.forgotPassword(email.trim());
      navigation.navigate('ResetPassword', { email: email.trim() });
    } catch (error) {
      let message = 'Something went wrong. Please try again.';
      if (error instanceof Error) {
        message = error.message;
      }
      Alert.alert('Error', message);
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
                  <Ionicons name="lock-closed-outline" size={28} color={colors.primary[500]} />
                </View>
                <Text className="text-xl font-bold text-text-primary mb-1">
                  Reset Your Password
                </Text>
                <Text className="text-sm text-text-secondary text-center leading-[20px]">
                  Enter your email and we&apos;ll send you a 6-digit code
                </Text>
              </View>

              {/* Form */}
              <View className="mb-4">
                <Input
                  label="Email"
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
                  title="Send reset code"
                  onPress={handleSubmit}
                  loading={isLoading}
                  fullWidth
                  style={glowButtonStyle}
                  testID="send-code-button"
                />
              </View>

              {/* Back to Login */}
              <View className="flex-row justify-center items-center gap-1 pt-2">
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
