// ABOUTME: Waiting screen for accounts that cannot sign in yet — unconfirmed address, or awaiting review
// ABOUTME: Mirrors the web PendingApproval: two different situations, two different next actions

import React, { useState } from 'react';
import {
  View,
  Text,

  Image,
  ScrollView,
  type ImageStyle,
  type ViewStyle,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { LinearGradient } from 'expo-linear-gradient';
import { Button } from '../../components/ui';
import { spacing, glassCard, gradients } from '../../constants/theme';
import { useRouter } from 'expo-router';
import { useAuth } from '../../contexts/AuthContext';
import { authApi } from '../../services/api';

// Logo style (pixel-specific dimensions)
const logoStyle: ImageStyle = { width: 100, height: 100, marginBottom: spacing.md };

// Glassmorphism card style
const cardStyle: ViewStyle = {
  ...glassCard,
  borderRadius: 16,
  overflow: 'hidden',
};

// Step badge style
const stepBadgeStyle: ViewStyle = {
  width: 24,
  height: 24,
  borderRadius: 12,
  alignItems: 'center',
  justifyContent: 'center',
  marginRight: spacing.sm,
};

/**
 * Shown to a signed-in user who cannot proceed yet. Two distinct situations land
 * here and they are not interchangeable:
 *
 * - **Address not confirmed** — the ball is in the user's court, so the screen
 *   leads with that and offers a resend. Telling someone to wait for an
 *   administrator when the real blocker is an unopened email is how people give
 *   up on a product.
 * - **Confirmed, awaiting review** — the ball is with an operator, so the screen
 *   says so and confirms the address is done.
 *
 * `email_verified` is optional deliberately: absent means the server did not
 * resolve it, not that the address is unconfirmed. Only an explicit `false`
 * switches to confirm-your-email mode.
 */
export function PendingApprovalScreen() {
  const router = useRouter();
  const { user } = useAuth();
  const [resendState, setResendState] = useState<'idle' | 'sending' | 'sent' | 'failed'>('idle');
  const needsEmailConfirmation = user?.email_verified === false;

  const handleResend = async () => {
    if (!user?.email || resendState === 'sending') return;
    setResendState('sending');
    try {
      await authApi.resendVerification(user.email);
      setResendState('sent');
    } catch {
      setResendState('failed');
    }
  };

  return (
    <SafeAreaView className="flex-1 bg-background-primary">
      <ScrollView
        contentContainerStyle={{ flexGrow: 1, justifyContent: 'center', paddingHorizontal: spacing.lg, paddingVertical: spacing.xl }}
      >
        {/* Glassmorphism Card Container */}
        <View style={cardStyle}>
          {/* Gradient accent bar at top */}
          <LinearGradient
            colors={gradients.violetCyan as [string, string]}
            start={{ x: 0, y: 0 }}
            end={{ x: 1, y: 0 }}
            style={{ height: 3, width: '100%' }}
          />

          <View className="px-6 py-8">
            {/* Pierre Logo */}
            <View className="items-center">
              <Image
                source={require('../../../assets/dravr-logo.png')}
                style={logoStyle}
                resizeMode="contain"
              />
            </View>

            {/* Message */}
            <Text className="text-xl font-bold text-text-primary text-center mb-3">
              {needsEmailConfirmation ? 'Confirm your email' : 'Account Pending Approval'}
            </Text>
            {needsEmailConfirmation ? (
              <Text className="text-sm text-text-secondary text-center leading-5 mb-4">
                We sent a confirmation link to your inbox. Open it to finish setting up your
                account — check your spam folder if it hasn&apos;t arrived.
              </Text>
            ) : (
              <>
                <Text className="text-sm text-text-secondary text-center leading-5 mb-2">
                  Thank you for registering with Dravr! Your account is currently
                  being reviewed by our team.
                </Text>
                <Text className="text-sm text-text-secondary text-center leading-5 mb-4">
                  You'll receive an email notification once your account has been
                  approved and is ready to use.
                </Text>
              </>
            )}

            {needsEmailConfirmation && (
              <View className="mb-4">
                <Button
                  title={resendState === 'sending' ? 'Sending…' : 'Send the link again'}
                  onPress={() => void handleResend()}
                  disabled={resendState === 'sending'}
                  fullWidth
                />
                {resendState === 'sent' && (
                  <Text className="text-xs text-text-tertiary text-center mt-2">
                    Sent. Give it a minute, then check your inbox and spam folder.
                  </Text>
                )}
                {resendState === 'failed' && (
                  <Text className="text-xs text-error text-center mt-2">
                    Couldn&apos;t send it just now. Try again in a moment.
                  </Text>
                )}
              </View>
            )}

            {/* Info Box with glassmorphism */}
            <View className="bg-background-tertiary rounded-xl p-4 mb-6 border border-border-subtle">
              <Text className="text-base font-semibold text-text-primary mb-3">
                What happens next?
              </Text>
              <View className="flex-row items-center mb-3">
                <LinearGradient
                  colors={gradients.violetCyan as [string, string]}
                  style={stepBadgeStyle}
                >
                  <Text className="text-on-surface text-xs font-bold">1</Text>
                </LinearGradient>
                <Text className="flex-1 text-sm text-text-secondary">
                  {needsEmailConfirmation
                    ? 'Open the confirmation link we emailed you'
                    : 'Our team reviews your registration'}
                </Text>
              </View>
              <View className="flex-row items-center mb-3">
                <LinearGradient
                  colors={gradients.violetCyan as [string, string]}
                  style={stepBadgeStyle}
                >
                  <Text className="text-on-surface text-xs font-bold">2</Text>
                </LinearGradient>
                <Text className="flex-1 text-sm text-text-secondary">
                  {needsEmailConfirmation
                    ? 'Your account activates as soon as it is confirmed'
                    : "You'll receive an approval email"}
                </Text>
              </View>
              <View className="flex-row items-center">
                <LinearGradient
                  colors={gradients.violetCyan as [string, string]}
                  style={stepBadgeStyle}
                >
                  <Text className="text-on-surface text-xs font-bold">3</Text>
                </LinearGradient>
                <Text className="flex-1 text-sm text-text-secondary">
                  Sign in and connect your fitness accounts
                </Text>
              </View>
            </View>

            {/* Back to Login */}
            <Button
              title="Back to Sign In"
              onPress={() => router.replace('/(auth)/login')}
              variant="secondary"
              fullWidth
              style={{ marginBottom: spacing.md }}
            />

          </View>
        </View>
      </ScrollView>
    </SafeAreaView>
  );
}
