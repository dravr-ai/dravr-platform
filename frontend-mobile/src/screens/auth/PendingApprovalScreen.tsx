// ABOUTME: Waiting screen for accounts that cannot sign in yet — unconfirmed address, or awaiting review
// ABOUTME: Mirrors the web PendingApproval: two different situations, two different next actions; plain page, no card shell

import React, { useState } from 'react';
import {
  View,
  Text,

  Image,
  ScrollView,
  TouchableOpacity,
  type ImageStyle,
  type ViewStyle,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { Button } from '../../components/ui';
import { spacing, useThemeColors } from '../../constants/theme';
import { useLocalSearchParams, useRouter } from 'expo-router';
import { useAuth } from '../../contexts/AuthContext';
import { authApi, userApi } from '../../services/api';
import { describeApiError, describeEmailTombstone } from '@pierre/ui-logic';
import { useTranslation } from '@pierre/i18n';

// Logo style (pixel-specific dimensions)
const logoStyle: ImageStyle = {
  width: 100,
  height: 100,
  borderRadius: 25,
  marginBottom: spacing.md,
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
 *
 * Arriving straight from the register form, nobody is signed in yet, so the
 * form passes the server's own sentence as `message` — awaiting review, or
 * confirm the address to activate — and the screen shows it instead of
 * assuming a review.
 *
 * A third reason rides on either: the address belonged to a Dravr account that
 * was deleted (`email_tombstone`). Nobody reviews that one — the athlete
 * chooses to start fresh with it, once the address is confirmed as theirs.
 */
export function PendingApprovalScreen() {
  const colors = useThemeColors();
  const { t, language } = useTranslation();
  const router = useRouter();
  const { user, updateUser } = useAuth();
  const { message: registerMessage } = useLocalSearchParams<{ message?: string }>();
  const [resendState, setResendState] = useState<'idle' | 'sending' | 'sent' | 'failed'>('idle');
  const [clearing, setClearing] = useState(false);
  const [clearError, setClearError] = useState<string | null>(null);
  const needsEmailConfirmation = user?.email_verified === false;
  const tombstone = user?.email_tombstone;
  // A tombstoned address the athlete has proven: theirs to start fresh with.
  const offersClear = tombstone !== undefined && !needsEmailConfirmation;

  const handleClear = async () => {
    if (!tombstone || clearing) return;
    setClearing(true);
    setClearError(null);
    try {
      const { user_status } = await userApi.clearImportTombstone(tombstone.tombstone_id);
      // An approval rule may admit the account at once; otherwise it now waits
      // for an ordinary review.
      await updateUser({ user_status, email_tombstone: undefined });
    } catch (err) {
      setClearError(describeApiError(err, { t, fallbackKey: 'errors.unknown' }));
    } finally {
      setClearing(false);
    }
  };

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
    <SafeAreaView className="flex-1 bg-background-primary" testID="pending-approval-screen">
      <ScrollView
        contentContainerStyle={{ flexGrow: 1, justifyContent: 'center', paddingHorizontal: spacing.lg, paddingVertical: spacing.xl }}
      >
        <View className="px-6 py-8">
          {/* Pierre Logo */}
          <View className="items-center">
            <Image
              source={require('../../../assets/icon.png')}
              style={logoStyle}
              resizeMode="contain"
            />
          </View>

          {/* Message */}
          <Text className="text-xl font-bold text-text-primary text-center mb-3">
            {needsEmailConfirmation ? t('app.confirmYourEmail') : t('app.pendingApprovalTitle')}
          </Text>
          {needsEmailConfirmation ? (
            <>
              <Text className="text-sm text-text-secondary text-center leading-5 mb-4">
                {t('app.sentConfirmationLink')}
              </Text>
              {tombstone && (
                <Text
                  className="text-sm text-text-secondary text-center leading-5 mb-4"
                  testID="pending-email-tombstoned-verify-first"
                >
                  {t('auth.pendingEmailTombstonedVerifyFirst')}
                </Text>
              )}
            </>
          ) : tombstone ? (
            <Text className="text-sm text-text-secondary text-center leading-5 mb-4" testID="pending-email-tombstoned">
              {describeEmailTombstone(tombstone, { language, t })}
            </Text>
          ) : registerMessage ? (
            <Text className="text-sm text-text-secondary text-center leading-5 mb-4" testID="pending-register-message">
              {registerMessage}
            </Text>
          ) : (
            <>
              <Text className="text-sm text-text-secondary text-center leading-5 mb-2">
                {t('app.thanksForRegistering')}
              </Text>
              <Text className="text-sm text-text-secondary text-center leading-5 mb-4">
                {t('app.approvalEmailBlurb')}
              </Text>
            </>
          )}

          {needsEmailConfirmation && (
            <View className="mb-4">
              <Button
                title={resendState === 'sending' ? t('app.sending') : t('app.sendLinkAgain')}
                onPress={() => void handleResend()}
                disabled={resendState === 'sending'}
                fullWidth
                testID="pending-resend-button"
              />
              {resendState === 'sent' && (
                <Text className="text-xs text-text-tertiary text-center mt-2">
                  {t('app.sentCheckInbox')}
                </Text>
              )}
              {resendState === 'failed' && (
                <Text className="text-xs text-error text-center mt-2">
                  {t('app.couldNotSendJustNow')}
                </Text>
              )}
            </View>
          )}

          {offersClear && (
            <View className="mb-6">
              <Button
                title={t('auth.pendingEmailTombstonedClear')}
                onPress={() => void handleClear()}
                loading={clearing}
                fullWidth
                testID="pending-email-tombstoned-clear"
              />
              {clearError && (
                <Text className="text-xs text-error text-center mt-2">{clearError}</Text>
              )}
            </View>
          )}

          {/* Info Box with glassmorphism — nobody reviews a tombstoned address,
              so the review steps would promise a wait that is not coming. */}
          {!offersClear && (
          <View className="bg-background-tertiary rounded-xl p-4 mb-6 border border-border-faint">
            <Text className="text-base font-semibold text-text-primary mb-3">
              {t('app.whatHappensNext')}
            </Text>
            <View className="flex-row items-center mb-3">
              <View style={[stepBadgeStyle, { backgroundColor: colors.tokens.primaryContainer }]}>
                <Text className="text-xs font-bold" style={{ color: colors.tokens.onPrimaryContainer }}>1</Text>
              </View>
              <Text className="flex-1 text-sm text-text-secondary">
                {needsEmailConfirmation
                  ? t('app.openConfirmationLink')
                  : t('app.teamReviewsRegistration')}
              </Text>
            </View>
            <View className="flex-row items-center mb-3">
              <View style={[stepBadgeStyle, { backgroundColor: colors.tokens.primaryContainer }]}>
                <Text className="text-xs font-bold" style={{ color: colors.tokens.onPrimaryContainer }}>2</Text>
              </View>
              <Text className="flex-1 text-sm text-text-secondary">
                {needsEmailConfirmation
                  ? t('app.accountActivatesOnConfirm')
                  : t('app.youllReceiveApproval')}
              </Text>
            </View>
            <View className="flex-row items-center">
              <View style={[stepBadgeStyle, { backgroundColor: colors.tokens.primaryContainer }]}>
                <Text className="text-xs font-bold" style={{ color: colors.tokens.onPrimaryContainer }}>3</Text>
              </View>
              <Text className="flex-1 text-sm text-text-secondary">
                {t('app.signInAndConnect')}
              </Text>
            </View>
          </View>
          )}

          {/* Back to Login — a quiet link, matching the other auth screens rather
              than a full button: this is the escape hatch, not the primary action
              on a screen whose primary action is "wait" (or, for the unconfirmed
              case, the resend button above). */}
          <View className="items-center">
            <TouchableOpacity
              onPress={() => router.replace('/(auth)/login')}
              testID="pending-back-to-sign-in-link"
            >
              <Text className="text-sm font-semibold text-primary">{t('app.backToSignInTitle')}</Text>
            </TouchableOpacity>
          </View>

        </View>
      </ScrollView>
    </SafeAreaView>
  );
}
