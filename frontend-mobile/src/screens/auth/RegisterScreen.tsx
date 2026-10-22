// ABOUTME: Registration screen for new user signup
// ABOUTME: Reads the sign-up config first: a paused notice when closed, a Turnstile check when keyed; no card shell

import React, { useCallback, useRef, useState } from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  Alert,
  type TextInput,
  type ViewStyle,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useQuery } from '@tanstack/react-query';
import {
  classifyApiError,
  describeApiError,
  isInvalidSignupCode,
  useAsyncAction,
} from '@pierre/ui-logic';
import { QUERY_KEYS } from '@pierre/shared-constants';
import { useAuth } from '../../contexts/AuthContext';
import { Button, FormScrollView, Input } from '../../components/ui';
import { BrandLockup } from '../../components/ui/BrandLockup';
import { TurnstileWebView, type TurnstileWebViewHandle } from '../../components/auth/TurnstileWebView';
import { authApi } from '../../services/api';
import { spacing } from '../../constants/theme';
import { useRouter } from 'expo-router';
import { useTranslation } from '@pierre/i18n';
import { signupCodeOrUndefined } from '../../utils/signupCode';

/**
 * The sentence an alert shows for a failed registration.
 *
 * The sign-up guards answer with a sentence of their own — paused or past
 * the day's ceiling (429 QuotaExceeded), too many sign-ups from this network
 * (429 RateLimitExceeded), the human check failed (403) — and
 * `describeApiError` would replace them with a generic "failed", which is
 * wrong for a deliberate pause.
 */
function describeRegisterError(error: unknown, t: (key: string) => string): string {
  const { status, detail } = classifyApiError(error);
  if (detail && (status === 429 || status === 403)) {
    return detail;
  }
  return describeApiError(error, { t, fallbackKey: 'app.registrationFailedLower' });
}

export function RegisterScreen() {
  const { t } = useTranslation();
  const router = useRouter();
  const { register } = useAuth();
  const [displayName, setDisplayName] = useState('');
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [confirmPassword, setConfirmPassword] = useState('');
  // A signup code is optional and most athletes have none, so its field
  // stays folded behind a link until asked for.
  const [showSignupCode, setShowSignupCode] = useState(false);
  const [signupCode, setSignupCode] = useState('');
  const [errors, setErrors] = useState<{
    displayName?: string;
    email?: string;
    password?: string;
    confirmPassword?: string;
    signupCode?: string;
  }>({});
  const [turnstileToken, setTurnstileToken] = useState<string | null>(null);
  const turnstileRef = useRef<TurnstileWebViewHandle>(null);

  // Whether sign-up is open and which bot check to render. A server that does
  // not answer this keeps the plain form: it is the one that enforces the
  // guards, and the form must not vanish because a read failed.
  const signupConfigQuery = useQuery({
    queryKey: QUERY_KEYS.signup.config(),
    queryFn: () => authApi.getSignupConfig(),
    retry: 1,
  });
  const signupPaused = signupConfigQuery.data?.open === false;
  const turnstileSiteKey = signupConfigQuery.data?.turnstile_site_key ?? null;
  const awaitingTurnstile = turnstileSiteKey !== null && turnstileToken === null;
  const handleTurnstileLoadError = useCallback(() => {
    Alert.alert(t('app.registrationFailedTitle'), t('errors.network'));
  }, [t]);

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
    action: () =>
      register(
        email.trim(),
        password,
        displayName.trim(),
        turnstileToken ?? undefined,
        signupCodeOrUndefined(signupCode),
      ),
    onSuccess: (response) => {
      // The server's sentence (awaiting review, or confirm the address to
      // activate) goes to the waiting screen rather than a fixed one.
      router.replace({ pathname: '/(auth)/pending-approval', params: { message: response.message } });
    },
    onError: (error: unknown) => {
      // A code the server will not take creates nothing: say so next to the
      // field, where the athlete fixes it, rather than in a dismissable alert.
      if (isInvalidSignupCode(error)) {
        setErrors((current) => ({ ...current, signupCode: t('auth.signupCodeInvalid') }));
      } else {
        Alert.alert(t('app.registrationFailedTitle'), describeRegisterError(error, t));
      }
      // A Turnstile token is single-use: the next attempt needs a fresh one.
      turnstileRef.current?.reset();
    },
    successResetDelay: 0,
    errorResetDelay: 0,
  });

  // Chains focus down the form: `returnKeyType` only relabels the return key,
  // so without refs to move the caret the athlete must aim at each next field
  // with the keyboard already covering it (carnet#353).
  const emailRef = useRef<TextInput>(null);
  const passwordRef = useRef<TextInput>(null);
  const confirmRef = useRef<TextInput>(null);

  const handleRegister = () => {
    if (signupConfigQuery.isLoading || awaitingTurnstile) return;
    if (!validateForm()) return;
    registerAction.execute();
  };

  const submitButtonStyle: ViewStyle = {
    marginTop: spacing.md,
  };

  return (
    <SafeAreaView className="flex-1 bg-background-primary" testID="register-screen">
        <FormScrollView
          contentContainerStyle={{ flexGrow: 1, justifyContent: 'center', paddingHorizontal: spacing.lg, paddingVertical: spacing.xl }}
        >
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

            {signupPaused ? (
              <View
                className="mb-4 rounded-xl border border-border-faint bg-background-tertiary px-4 py-3"
                testID="register-signup-paused"
              >
                <Text className="text-sm text-text-primary text-center leading-[20px]">
                  {t('auth.signupPaused')}
                </Text>
              </View>
            ) : (
            /* Registration Form */
            <View className="mb-4">
              <Input
                label={t('app.displayName')}
                placeholder={t('app.howShouldWeCallYou')}
                value={displayName}
                onChangeText={setDisplayName}
                autoCapitalize="words"
                error={errors.displayName}
                returnKeyType="next"
                onSubmitEditing={() => emailRef.current?.focus()}
                blurOnSubmit={false}
                testID="register-display-name-input"
              />

              <Input
                ref={emailRef}
                label={t('common.email')}
                placeholder="you@example.com"
                value={email}
                onChangeText={setEmail}
                keyboardType="email-address"
                autoCapitalize="none"
                autoCorrect={false}
                error={errors.email}
                returnKeyType="next"
                onSubmitEditing={() => passwordRef.current?.focus()}
                blurOnSubmit={false}
                testID="register-email-input"
              />

              <Input
                ref={passwordRef}
                label={t('common.password')}
                placeholder={t('app.minEightChars')}
                value={password}
                onChangeText={setPassword}
                secureTextEntry
                showPasswordToggle
                error={errors.password}
                returnKeyType="next"
                onSubmitEditing={() => confirmRef.current?.focus()}
                blurOnSubmit={false}
                testID="register-password-input"
              />

              <Input
                ref={confirmRef}
                label={t('app.confirmPassword')}
                placeholder={t('app.reenterPassword')}
                value={confirmPassword}
                onChangeText={setConfirmPassword}
                secureTextEntry
                showPasswordToggle
                error={errors.confirmPassword}
                returnKeyType="go"
                onSubmitEditing={handleRegister}
                testID="register-confirm-password-input"
              />

              {showSignupCode ? (
                <View testID="register-signup-code">
                  <Input
                    label={t('auth.signupCodeLabel')}
                    placeholder={t('auth.signupCodePlaceholder')}
                    value={signupCode}
                    onChangeText={(value) => {
                      setSignupCode(value);
                      if (errors.signupCode) {
                        setErrors((current) => ({ ...current, signupCode: undefined }));
                      }
                    }}
                    autoCapitalize="characters"
                    autoCorrect={false}
                    error={errors.signupCode}
                    returnKeyType="go"
                    onSubmitEditing={handleRegister}
                    testID="register-signup-code-input"
                  />
                  <Text className="text-xs text-text-tertiary mb-2">{t('auth.signupCodeHint')}</Text>
                </View>
              ) : (
                <TouchableOpacity
                  onPress={() => setShowSignupCode(true)}
                  className="self-start py-2 mb-2"
                  testID="register-signup-code-toggle"
                >
                  <Text className="text-sm font-semibold text-primary">{t('auth.haveSignupCode')}</Text>
                </TouchableOpacity>
              )}

              {turnstileSiteKey && (
                <TurnstileWebView
                  ref={turnstileRef}
                  siteKey={turnstileSiteKey}
                  onToken={setTurnstileToken}
                  onLoadError={handleTurnstileLoadError}
                />
              )}

              <Button
                title={t('app.createAccount')}
                onPress={handleRegister}
                loading={registerAction.isLoading}
                disabled={signupConfigQuery.isLoading || awaitingTurnstile}
                fullWidth
                style={submitButtonStyle}
                testID="register-submit-button"
              />
            </View>
            )}

            {/* Login Link */}
            <View className="flex-row justify-center items-center gap-1 pt-2">
              <Text className="text-sm text-text-secondary">{t('app.alreadyHaveAccount')}</Text>
              <TouchableOpacity onPress={() => router.replace('/(auth)/login')} testID="register-login-link">
                <Text className="text-sm font-semibold text-primary">{t('common.login')}</Text>
              </TouchableOpacity>
            </View>
          </View>
        </FormScrollView>
    </SafeAreaView>
  );
}
