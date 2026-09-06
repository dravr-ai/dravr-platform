// ABOUTME: Login screen with email/password and Google Sign-In authentication
// ABOUTME: The phone's half of DESIGN.md §5 "Auth and onboarding" — tint page, white form sheet, both schemes

import React, { useState } from 'react';
import {
  View,
  Text,
  ScrollView,
  TouchableOpacity,
  Alert,
  ActivityIndicator,
  type ViewStyle,
  type TextStyle,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { StatusBar } from 'expo-status-bar';
import { useAuth } from '../../contexts/AuthContext';
import { useTheme, useThemeColors } from '../../contexts/ThemeContext';
import { Button, Input } from '../../components/ui';
import { BrandLockup } from '../../components/ui/BrandLockup';
import { PROVIDER_COLORS, spacing } from '../../constants/theme';
import {
  FIREBASE_NOT_CONFIGURED,
  GOOGLE_SIGNIN_UNAVAILABLE,
  NO_GOOGLE_ID_TOKEN,
  isFirebaseEnabled,
  signInWithGoogle,
} from '../../firebase';
import { AntDesign } from '@expo/vector-icons';
import { useRouter } from 'expo-router';
import { useTranslation } from '@pierre/i18n';

/**
 * The catalogue key for a Google sign-in failure.
 *
 * `signInWithGoogle` throws codes rather than sentences, because this screen
 * used to show `error.message` to the athlete and every one of those messages
 * was English (carnet#207). An unrecognised failure — the native module's own
 * error — falls back to the generic wording rather than leaking its text.
 */
function googleFailureKey(error: unknown): string {
  const code = error instanceof Error ? error.message : '';
  switch (code) {
    case GOOGLE_SIGNIN_UNAVAILABLE:
      return 'errors.googleSignInUnavailable';
    case FIREBASE_NOT_CONFIGURED:
      return 'errors.firebaseNotConfigured';
    case NO_GOOGLE_ID_TOKEN:
      return 'errors.noGoogleIdToken';
    default:
      return 'auth.googleSignInFailed';
  }
}

export function LoginScreen() {
  const { t } = useTranslation();
  const router = useRouter();
  const { scheme } = useTheme();
  const colors = useThemeColors();
  const { tokens } = colors;
  const isDark = scheme === 'dark';

  /**
   * The two sheets, the phone's version of the web login's aside-and-form.
   *
   * Light pairs the sage tint with a white card, so the brand ground and the
   * form read as different things; dark pairs the paper-dark canvas with a
   * container tier ABOVE it, because a `lowest` card sinks below a near-black
   * ground while the tint becomes a dense green wall. Before this the screen
   * was a hardcoded `#00241a → #0d3b2e` gradient — the retired v1 primary —
   * under a card pinned to light, so it ignored the athlete's appearance
   * setting entirely and shipped the one surface in the app that could not
   * be dark.
   */
  const pageGround = isDark ? tokens.surface : tokens.primaryContainer;
  const cardGround = isDark ? tokens.surfaceContainerHigh : tokens.surfaceContainerLowest;
  const { login, loginWithFirebase } = useAuth();
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [isLoading, setIsLoading] = useState(false);
  const [isGoogleLoading, setIsGoogleLoading] = useState(false);
  const [errors, setErrors] = useState<{ email?: string; password?: string }>({});

  const validateForm = () => {
    const newErrors: { email?: string; password?: string } = {};

    if (!email.trim()) {
      newErrors.email = t('validation.emailRequired');
    } else if (!/\S+@\S+\.\S+/.test(email)) {
      newErrors.email = t('validation.email');
    }

    if (!password) {
      newErrors.password = t('validation.passwordRequired');
    }

    setErrors(newErrors);
    return Object.keys(newErrors).length === 0;
  };

  const handleLogin = async () => {
    if (!validateForm()) return;

    setIsLoading(true);
    try {
      await login(email.trim(), password);
      // Navigation is handled by auth state change in root layout auth gating
      // If user is pending, the auth guard redirects to PendingApproval screen
    } catch (error) {
      let message = 'Login failed. Please try again.';
      if (error instanceof Error) {
        // Parse API error responses
        if (error.message.includes('400') || error.message.includes('invalid')) {
          message = 'Invalid email or password. Please check your credentials.';
        } else if (error.message.includes(t('app.networkTitle'))) {
          message = 'Network error. Please check your connection.';
        } else {
          message = error.message;
        }
      }
      Alert.alert(t('app.loginFailedTitle'), message);
    } finally {
      setIsLoading(false);
    }
  };

  const handleGoogleSignIn = async () => {
    setIsGoogleLoading(true);
    try {
      const result = await signInWithGoogle();
      if (result) {
        await loginWithFirebase(result.idToken);
        // Navigation handled by auth state change
      }
      // A null result means the user dismissed the native sheet — no alert.
    } catch (error) {
      Alert.alert(t('app.signInFailedTitle'), t(googleFailureKey(error)));
    } finally {
      setIsGoogleLoading(false);
    }
  };

  // The headline that says what the product is.
  const heroHeadlineStyle: TextStyle = {
    fontFamily: 'SpaceGrotesk_SemiBold',
    fontSize: 26,
    lineHeight: 32,
    color: tokens.onSurface,
  };

  const heroLeadStyle: TextStyle = {
    fontFamily: 'PlusJakartaSans',
    fontSize: 14,
    lineHeight: 22,
    color: tokens.onSurfaceVariant,
  };

  // The form sheet. Hairline, no shadow — DESIGN.md §4: hairlines lift,
  // shadows float, and nothing at rest floats.
  const cardStyle: ViewStyle = {
    backgroundColor: cardGround,
    borderRadius: 20,
    borderWidth: 1,
    borderColor: colors.border.default,
    overflow: 'hidden',
  };

  const submitButtonStyle: ViewStyle = { marginTop: spacing.md };

  return (
    <View className="flex-1" testID="login-screen" style={{ backgroundColor: pageGround }}>
      <StatusBar style={isDark ? 'light' : 'dark'} />
      <SafeAreaView className="flex-1" edges={['top', 'bottom']}>
        <ScrollView
          contentContainerStyle={{ flexGrow: 1, paddingHorizontal: spacing.lg, paddingVertical: spacing.xl }}
          keyboardShouldPersistTaps="handled"
          automaticallyAdjustKeyboardInsets
          testID="login-scroll-view"
        >
          {/* Editorial hero band — brand moment at the top */}
          <View style={{ marginTop: spacing.md, marginBottom: spacing.xl }}>
            {/* One lockup component, the same one the chat tab header wears —
                the mark and the wordmark are not re-specified per screen. */}
            <View style={{ marginBottom: spacing.lg }}>
              <BrandLockup size={40} testID="login-lockup" />
            </View>
            {/* One sentence, one pair of keys — the same two the web aside
                uses. The phone joins them with a space and lets the line
                break fall where the width says: a hard `\n` here fought the
                wrap and rendered the designed two lines as a ragged three at
                390pt, and a phone-only copy of the lead is a second source
                for the same sentence. Web keeps its `<br>`, where the aside
                only ever renders at >=1024px and the break is a design choice. */}
            <Text style={[heroHeadlineStyle, { marginBottom: spacing.sm }]}>
              {`${t('auth.taglineLead')} ${t('auth.taglineTail')}`}
            </Text>
            <Text style={heroLeadStyle}>
              {t('app.heroBlurb')}
            </Text>
          </View>

          {/* The form sheet on the brand ground. Every element inside reads
              the live palette, so the whole screen follows the athlete's
              appearance setting. */}
          <View style={cardStyle} testID="login-card">
            <View className="px-6 py-7">
              <View className="mb-5">
                <Text
                  style={{
                    fontFamily: 'SpaceGrotesk_SemiBold',
                    fontSize: 24,
                    marginBottom: 4,
                    color: tokens.onSurface,
                  }}
                >
                  {t('common.login')}
                </Text>
                <Text style={{ fontSize: 14, color: tokens.onSurfaceVariant }}>
                  {t('app.welcomeBack')}
                </Text>
              </View>

              {/* Login Form */}
              <View className="mb-2">
                <Input
                  label={t('common.email')}
                  placeholder="you@example.com"
                  value={email}
                  onChangeText={setEmail}
                  keyboardType="email-address"
                  autoCapitalize="none"
                  autoCorrect={false}
                  error={errors.email}
                  testID="email-input"
                />

                <Input
                  label={t('common.password')}
                  placeholder={t('app.enterYourPassword')}
                  value={password}
                  onChangeText={setPassword}
                  secureTextEntry
                  showPasswordToggle
                  returnKeyType="go"
                  onSubmitEditing={handleLogin}
                  error={errors.password}
                  testID="password-input"
                />

                <TouchableOpacity
                  onPress={() => router.push('/(auth)/forgot-password')}
                  className="self-end mb-2"
                  testID="forgot-password-link"
                >
                  <Text style={{ fontSize: 12, color: tokens.primary }}>
                    {t('app.forgotPasswordLink')}
                  </Text>
                </TouchableOpacity>

                <Button
                  title={t('common.login')}
                  onPress={handleLogin}
                  loading={isLoading}
                  fullWidth
                  style={submitButtonStyle}
                  testID="login-button"
                />

                {/* Google Sign-In - only show when Firebase is configured */}
                {isFirebaseEnabled() && (
                  <>
                    <View className="flex-row items-center my-5">
                      <View
                        style={{ flex: 1, height: 1, backgroundColor: colors.border.default }}
                      />
                      <Text
                        className="px-3"
                        style={{ fontSize: 13, color: tokens.onSurfaceVariant }}
                      >
                        {t('app.orContinueWith')}
                      </Text>
                      <View
                        style={{ flex: 1, height: 1, backgroundColor: colors.border.default }}
                      />
                    </View>

                    <TouchableOpacity
                      onPress={handleGoogleSignIn}
                      disabled={isGoogleLoading}
                      testID="google-signin-button"
                      activeOpacity={0.7}
                      style={{
                        flexDirection: 'row',
                        alignItems: 'center',
                        justifyContent: 'center',
                        gap: 10,
                        paddingVertical: 12,
                        paddingHorizontal: 20,
                        borderRadius: 12,
                        borderWidth: 1,
                        borderColor: colors.border.strong,
                        backgroundColor: 'transparent',
                      }}
                    >
                      {isGoogleLoading ? (
                        <ActivityIndicator size="small" color={tokens.onSurface} />
                      ) : (
                        <AntDesign name="google" size={20} color={PROVIDER_COLORS.google} />
                      )}
                      <Text
                        style={{
                          fontSize: 15,
                          fontWeight: '500',
                          color: tokens.onSurface,
                        }}
                      >
                        {isGoogleLoading ? t('app.signingIn') : t('app.continueWithGoogle')}
                      </Text>
                    </TouchableOpacity>
                  </>
                )}
              </View>

              {/* Register Link */}
              <View className="flex-row justify-center items-center gap-1 pt-4">
                <Text style={{ fontSize: 13, color: tokens.onSurfaceVariant }}>
                  {t('app.noAccountYet')}
                </Text>
                <TouchableOpacity onPress={() => router.push('/(auth)/register')}>
                  <Text
                    style={{ fontSize: 13, fontWeight: '600', color: tokens.primary }}
                  >
                    {t('app.createOne')}
                  </Text>
                </TouchableOpacity>
              </View>
            </View>
          </View>

          {/* The four pillars an athlete can ask about, sentence case like
              the web aside's — no tracked caps, which v2 retired. */}
          <View
            className="flex-row items-center justify-center"
            style={{ gap: spacing.sm, marginTop: spacing.xl }}
          >
            {[t('app.activity'), t('app.nutrition'), t('app.recovery'), t('app.mobility')].map((pillar, i) => (
              <React.Fragment key={pillar}>
                {i > 0 && (
                  <Text style={{ color: tokens.outline }} aria-hidden>
                    ·
                  </Text>
                )}
                <Text style={{ fontFamily: 'PlusJakartaSans', fontSize: 13, color: tokens.onSurfaceVariant }}>
                  {pillar}
                </Text>
              </React.Fragment>
            ))}
          </View>
        </ScrollView>
      </SafeAreaView>
    </View>
  );
}
