// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Sciotte credential login modal for mobile — collects email/password for headless Chrome login
// ABOUTME: Supports Google/Apple/email methods with 2FA (OTP, phone tap, number match)

import React, { useState, useEffect, useCallback } from 'react';
import {
  View,
  Text,
  TextInput,
  TouchableOpacity,
  Modal,
  ActivityIndicator,
  KeyboardAvoidingView,
  Platform,
  ScrollView,
  Alert,
  useWindowDimensions,
} from 'react-native';
import * as WebBrowser from 'expo-web-browser';
import * as Linking from 'expo-linking';
import { LinearGradient } from 'expo-linear-gradient';
import { Mail, ArrowLeft, Eye, EyeOff, Shield, CheckCircle2, AlertCircle, X, Key } from 'lucide-react-native';
import { PROVIDER_COLORS, useCardStyle, useThemeColors } from '../constants/theme';
import { oauthApi } from '../services/api';
import { getOAuthCallbackUrl } from '../utils/oauth';
import { StravaLogo, GarminLogo, TrainingPeaksLogo, CorosLogo, GoogleLogo, AppleLogo } from './icons/BrandIcons';
import type { SciotteTarget } from '@pierre/shared-types';
import { OAuthAppSetupModal } from './OAuthAppSetupModal';
import { ProviderNotice } from './ProviderNotice';
import { PROVIDER_NOTICES, type ProviderNoticeKeys } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { PROVIDER_BRAND } from '../constants/brands';
import { describeApiError } from '@pierre/ui-logic';

type LoginPhase =
  | 'choose'
  | 'credentials'
  | 'logging-in'
  | 'two-factor'
  | 'waiting-approval'
  | 'number-match'
  | 'otp'
  | 'success'
  | 'error';

type LoginMethod = 'email' | 'google' | 'apple';

interface SciotteLoginModalProps {
  visible: boolean;
  onClose: () => void;
  onConnected: () => void;
  target: SciotteTarget;
  /**
   * The account has not yet accepted this provider's exposure notice (the
   * row's `consent_required`): the notice and its required checkbox come
   * before the credentials, and the login carries the acceptance.
   */
  consentRequired?: boolean;
}

/**
 * The ink of every label and glyph drawn ON a brand plate. The plates are the
 * providers' colours and do not move with the scheme, so neither does the ink
 * that sits on them.
 */
const PLATE_INK = '#FFFFFF';

/**
 * The colours this login draws in — third-party colours, not Boreal tokens
 * (DESIGN.md §2). Each brand hex is read from `PROVIDER_COLORS`, which names
 * its source.
 *
 * `primary` is the brand hex. It fills the solid tile behind a white glyph,
 * which answers to the 3:1 icon floor: Strava 3.40:1, Garmin 4.496:1,
 * TrainingPeaks 7.59:1, COROS 3.93:1.
 *
 * `gradient` is the plate a white label sits on (Sign In, Verify, Try Again:
 * 16pt, so the 4.5:1 text floor), and both of its stops clear 4.5:1 against
 * white. Along a straight sRGB sweep the luminance never rises above the
 * lighter stop's, so the label clears the plate wherever it lands. The near
 * stop is the brand hex darkened (every sRGB channel scaled toward black)
 * only as far as white needs; the far stop is the brand hex at 75 %.
 *
 * | Plate         | near stop                                | far stop          |
 * |---------------|------------------------------------------|-------------------|
 * | Strava        | `#D74102` 4.52:1, the hex at 85.5 %      | `#BD3902` 5.60:1  |
 * | Garmin        | `#007CC2` 4.50:1, one step off the hex   | `#005D92` 7.05:1  |
 * | TrainingPeaks | `#005695` 7.59:1, the hex itself         | `#004070` 10.67:1 |
 * | COROS         | `#E52437` 4.53:1, the hex at 92.5 %      | `#BA1D2C` 6.36:1  |
 *
 * White on the Strava hex alone is 3.40:1, on the Garmin hex 4.496:1 and on
 * the COROS hex 3.93:1, so none of those plates could start from its brand hex.
 *
 * Google and Apple are the sign-in methods of the Strava choice, drawn only
 * as the sweep around their rows. Google's far stop (`#3367D6`) is the value
 * the integration shipped with, not traced to a Google source; Apple's is its
 * white "Sign in with Apple" style fading to a light grey.
 */
const BRAND_COLORS = {
  strava: {
    primary: PROVIDER_COLORS.strava,
    gradient: ['#D74102', '#BD3902'] as [string, string],
  },
  garmin: {
    primary: PROVIDER_COLORS.garmin,
    gradient: ['#007CC2', '#005D92'] as [string, string],
  },
  trainingpeaks: {
    primary: PROVIDER_COLORS.trainingpeaks,
    gradient: [PROVIDER_COLORS.trainingpeaks, '#004070'] as [string, string],
  },
  coros: {
    primary: PROVIDER_COLORS.coros,
    gradient: ['#E52437', '#BA1D2C'] as [string, string],
  },
  google: {
    primary: PROVIDER_COLORS.google,
    gradient: [PROVIDER_COLORS.google, '#3367D6'] as [string, string],
  },
  apple: { primary: '#FFFFFF', gradient: ['#FFFFFF', '#E8E8E8'] as [string, string] },
};

interface MethodConfig {
  /** Corpus key for the row's title. Resolved at render — module scope has no hook. */
  titleKey: string;
  /** Corpus key for the e-mail field's placeholder. */
  emailPlaceholderKey: string;
  /** The provider's own name. A proper noun; the same in every locale. */
  brandName: string;
  renderIcon: (size: number) => React.ReactNode;
  brandGradient: [string, string];
  /** The tile behind the icon: a solid plate, so a white glyph clears it. */
  bgColor: string;
}

const METHOD_CONFIGS: Record<LoginMethod, MethodConfig> = {
  email: {
    titleKey: 'app.emailAndPassword',
    emailPlaceholderKey: 'app.emailAddress',
    brandName: '',
    renderIcon: (size: number) => <Mail size={size} color={PLATE_INK} />,
    brandGradient: BRAND_COLORS.strava.gradient,
    bgColor: BRAND_COLORS.strava.primary,
  },
  google: {
    titleKey: 'app.brandGoogle',
    emailPlaceholderKey: 'app.googleEmail',
    brandName: PROVIDER_BRAND.google,
    renderIcon: (size: number) => <GoogleLogo size={size} />,
    brandGradient: BRAND_COLORS.google.gradient,
    bgColor: '#FFFFFF',
  },
  apple: {
    titleKey: 'app.brandApple',
    emailPlaceholderKey: 'app.appleIdEmail',
    brandName: PROVIDER_BRAND.apple,
    renderIcon: (size: number) => <AppleLogo size={size} color={PLATE_INK} />,
    brandGradient: BRAND_COLORS.apple.gradient,
    bgColor: '#000000',
  },
};

/** How the credential login presents each target. */
interface TargetPreset {
  /** The provider's name in the header and progress copy. A proper noun. */
  brandKey: string;
  brandColor: { primary: string; gradient: [string, string] };
  renderLogo: (size: number) => React.ReactNode;
  /** Title and placeholder of the provider's own credential form. */
  titleKey: string;
  placeholderKey: string;
  /** What the provider signs in with — TrainingPeaks takes a username. */
  identifier: 'email' | 'username';
  /** No Google/Apple choice to make: straight to the provider's form. */
  directCredentials: boolean;
  /** The exposure notice shown while the account has not accepted it. */
  notice?: ProviderNoticeKeys;
}

const TARGET_PRESETS: Record<SciotteTarget, TargetPreset> = {
  strava: {
    brandKey: 'app.brandStrava',
    brandColor: BRAND_COLORS.strava,
    renderLogo: (size) => <StravaLogo size={size} color={PLATE_INK} />,
    titleKey: 'app.emailAndPassword',
    placeholderKey: 'app.emailAddress',
    identifier: 'email',
    directCredentials: false,
  },
  garmin: {
    brandKey: 'app.brandGarminConnect',
    brandColor: BRAND_COLORS.garmin,
    renderLogo: (size) => <GarminLogo size={size} color={PLATE_INK} />,
    titleKey: 'app.garminAccount',
    placeholderKey: 'app.garminEmail',
    identifier: 'email',
    directCredentials: true,
  },
  trainingpeaks: {
    brandKey: 'app.brandTrainingPeaks',
    brandColor: BRAND_COLORS.trainingpeaks,
    renderLogo: (size) => <TrainingPeaksLogo size={size} color={PLATE_INK} />,
    titleKey: 'app.trainingpeaksAccount',
    placeholderKey: 'app.trainingpeaksUsername',
    identifier: 'username',
    directCredentials: true,
    notice: PROVIDER_NOTICES.sciotte_trainingpeaks,
  },
  coros: {
    brandKey: 'app.brandCoros',
    brandColor: BRAND_COLORS.coros,
    renderLogo: (size) => <CorosLogo size={size} color={PLATE_INK} />,
    titleKey: 'app.corosAccount',
    placeholderKey: 'app.corosEmail',
    identifier: 'email',
    directCredentials: true,
    notice: PROVIDER_NOTICES.sciotte_coros,
  },
};

export function SciotteLoginModal({
  visible,
  onClose,
  onConnected,
  target,
  consentRequired = false,
}: SciotteLoginModalProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const cardStyle = useCardStyle();
  // The sheet's content scrolls within most of the screen, never less than
  // the 420 it always had: a provider notice above the form would otherwise
  // push the password and Log In below the fold.
  const { height: windowHeight } = useWindowDimensions();
  const contentMaxHeight = Math.max(420, Math.round(windowHeight * 0.65));
  const [phase, setPhase] = useState<LoginPhase>('choose');
  const [method, setMethod] = useState<LoginMethod>('email');
  const [status, setStatus] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [otpCode, setOtpCode] = useState('');
  const [twoFactorOptions, setTwoFactorOptions] = useState<Array<{ id: string; label: string }>>([]);
  const [showPassword, setShowPassword] = useState(false);
  const [matchNumber, setMatchNumber] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  // Toggles the BYO Strava OAuth-app setup modal. Strava-only — Garmin doesn't
  // expose a public OAuth app for end users (only sciotte's headless login).
  const [showStravaBYO, setShowStravaBYO] = useState(false);

  const preset = TARGET_PRESETS[target];
  const brandColor = preset.brandColor;
  // Brand names, not copy: identical in every locale.
  const platformName = t(preset.brandKey);
  // The notice is shown only while the account has not accepted it, and only
  // for a target that has one.
  const notice = consentRequired ? preset.notice : undefined;
  const [consentAccepted, setConsentAccepted] = useState(false);

  useEffect(() => {
    if (visible) {
      setPhase(TARGET_PRESETS[target].directCredentials ? 'credentials' : 'choose');
      setMethod('email');
      setConsentAccepted(false);
      setStatus('');
      setError(null);
      setEmail('');
      setPassword('');
      setOtpCode('');
      setIsLoading(false);
      setShowPassword(false);
    }
  }, [visible, target]);

  const selectMethod = (m: LoginMethod) => {
    setMethod(m);
    setPhase('credentials');
    setError(null);
  };

  // After the BYO setup modal saves, kick off the official Strava OAuth dance
  // through ASWebAuthenticationSession. Mirrors the web flow in
  // `SciotteLoginModal.onSaved` (see frontend/) — credentials persisted on
  // the server, OAuth consent screen handles the rest. The Sciotte modal
  // closes as soon as Strava redirects back with success.
  const launchStravaOAuthFromBYO = useCallback(async () => {
    setShowStravaBYO(false);
    try {
      const returnUrl = getOAuthCallbackUrl();
      const oauthResponse = await oauthApi.initMobileOAuth('strava', returnUrl);
      const result = await WebBrowser.openAuthSessionAsync(
        oauthResponse.authorization_url,
        returnUrl,
      );
      if (result.type === 'success' && result.url) {
        if (!result.url.startsWith(returnUrl)) {
          Alert.alert(t('app.connectionFailed'), t('app.unexpectedOauthCallback'));
          return;
        }
        const parsed = Linking.parse(result.url);
        const success = parsed.queryParams?.success === 'true';
        const oauthError = parsed.queryParams?.error as string | undefined;
        if (success) {
          onConnected();
          onClose();
        } else if (oauthError) {
          Alert.alert(t('app.connectionFailed'), t('app.stravaErrorReason', { reason: oauthError }));
        }
      }
      // type === 'cancel' / 'dismiss' — user backed out; no error to surface.
    } catch (err) {
      const message = describeApiError(err, { t, fallbackKey: 'app.stravaOauthLaunchFailed' });
      Alert.alert(t('app.connectionFailed'), message);
    }
  }, [onConnected, onClose, t]);

  const handleLogin = useCallback(async () => {
    if (!email || !password) return;
    if (notice && !consentAccepted) return;

    setIsLoading(true);
    setError(null);
    setPhase('logging-in');
    setStatus(t('app.signingInTo', { provider: platformName }));

    try {
      const data = await oauthApi.sciotteLogin({
        email,
        password,
        method,
        target,
        ...(notice ? { tos_consent: consentAccepted } : {}),
      });

      if (data.status === 'connected') {
        setPhase('success');
        setStatus(t('app.connectedBang'));
        onConnected();
        setTimeout(onClose, 1500);
      } else if (data.status === 'two_factor_choice') {
        setTwoFactorOptions(data.options || []);
        setPhase('two-factor');
        setStatus(t('app.verificationRequired'));
      } else if (data.status === 'otp_required') {
        setPhase('otp');
        setStatus(t('app.enterVerificationCode'));
        setOtpCode('');
      } else if (data.status === 'number_match') {
        setMatchNumber(data.number || null);
        setPhase('number-match');
        setStatus(t('app.confirmOnDevice'));
      } else {
        setError(data.error || t('auth.loginFailed'));
        setPhase('error');
      }
    } catch (err) {
      const message = describeApiError(err, { t, fallbackKey: 'auth.loginFailed' });
      setError(message);
      setPhase('error');
    } finally {
      setIsLoading(false);
    }
  }, [email, password, method, target, notice, consentAccepted, platformName, onClose, onConnected, t]);

  const handleSelect2FA = useCallback(async (optionId: string) => {
    setIsLoading(true);
    if (optionId === 'app') setPhase('waiting-approval');
    else if (optionId !== 'poll') setPhase('logging-in');
    setStatus(optionId === 'app' ? t('app.approveOnDevice') : t('app.verifying'));

    try {
      const data = await oauthApi.sciotteSelect2FA(optionId);

      if (data.status === 'connected') {
        setPhase('success');
        setStatus(t('app.connectedBang'));
        onConnected();
        setTimeout(onClose, 1500);
      } else if (data.status === 'otp_required') {
        setPhase('otp');
        setStatus(t('app.enterVerificationCode'));
        setOtpCode('');
      } else if (data.status === 'number_match') {
        setMatchNumber(data.number || null);
        setPhase('number-match');
        setStatus(t('app.confirmOnDevice'));
      } else {
        setError(data.error || t('app.verificationFailed'));
        setPhase('error');
      }
    } catch (err) {
      setError(describeApiError(err, { t, fallbackKey: 'app.verificationFailed' }));
      setPhase('error');
    } finally {
      setIsLoading(false);
    }
  }, [onClose, onConnected, t]);

  const [pollingStarted, setPollingStarted] = useState(false);
  useEffect(() => {
    if (phase === 'number-match' && matchNumber && !pollingStarted) {
      setPollingStarted(true);
      handleSelect2FA('poll');
    }
    if (phase !== 'number-match') {
      setPollingStarted(false);
    }
  }, [phase, matchNumber]); // eslint-disable-line react-hooks/exhaustive-deps

  const handleOtpSubmit = useCallback(async () => {
    if (!otpCode) return;

    setIsLoading(true);
    setPhase('logging-in');
    setStatus(t('app.verifyingCode'));

    try {
      const data = await oauthApi.sciotteSubmitOTP(otpCode);

      if (data.status === 'connected') {
        setPhase('success');
        setStatus(t('app.connectedBang'));
        onConnected();
        setTimeout(onClose, 1500);
      } else if (data.status === 'otp_required') {
        setPhase('otp');
        setOtpCode('');
      } else {
        setError(data.error || t('app.verificationFailed'));
        setPhase('error');
      }
    } catch (err) {
      setError(describeApiError(err, { t, fallbackKey: 'app.verificationFailed' }));
      setPhase('error');
    } finally {
      setIsLoading(false);
    }
  }, [otpCode, onClose, onConnected, t]);

  // A provider that signs in with its own credentials draws the form's mark on
  // its own colour, not the email row's Strava orange.
  const methodConfig = preset.directCredentials
    ? {
        ...METHOD_CONFIGS.email,
        titleKey: preset.titleKey,
        emailPlaceholderKey: preset.placeholderKey,
        bgColor: preset.brandColor.primary,
      }
    : METHOD_CONFIGS[method];
  const usesUsername = preset.identifier === 'username';
  const canSubmit = Boolean(email && password) && (!notice || consentAccepted);

  const canGoBack = phase === 'credentials' && !preset.directCredentials;

  const renderContent = () => {
    // Choose login method (Strava only)
    if (phase === 'choose') {
      return (
        <View className="gap-3">
          <Text className="text-sm text-text-secondary text-center mb-2">
            {t('app.chooseHowYouSignIn')} {platformName}
          </Text>

          {(['email', 'google', 'apple'] as LoginMethod[]).map((m) => {
            const cfg = METHOD_CONFIGS[m];
            return (
              <TouchableOpacity key={m} onPress={() => selectMethod(m)} activeOpacity={0.8}>
                <LinearGradient
                  colors={cfg.brandGradient}
                  start={{ x: 0, y: 0 }}
                  end={{ x: 1, y: 0 }}
                  style={{ borderRadius: 14, padding: 1 }}
                >
                  <View
                    className="flex-row items-center rounded-[13px] px-5 py-4"
                    style={{ backgroundColor: m === 'apple' ? '#FFFFFF' : colors.background.secondary }}
                  >
                    <View
                      className="w-10 h-10 rounded-xl items-center justify-center mr-4"
                      style={{ backgroundColor: cfg.bgColor }}
                    >
                      {cfg.renderIcon(20)}
                    </View>
                    <View className="flex-1">
                      <Text
                        className="text-base font-semibold"
                        style={{ color: m === 'apple' ? '#000000' : colors.text.primary }}
                      >
                        {m === 'email'
                          ? t('app.providerEmailLabel', { provider: platformName })
                          : t('app.continueWithProvider', { provider: cfg.brandName })}
                      </Text>
                      <Text
                        className="text-xs mt-0.5"
                        style={{ color: m === 'apple' ? '#666666' : colors.text.tertiary }}
                      >
                        {m === 'email'
                          ? t('app.signInWithEmailPassword')
                          : m === 'google'
                            ? t('app.useGoogleAccount')
                            : t('app.useAppleId')}
                      </Text>
                    </View>
                  </View>
                </LinearGradient>
              </TouchableOpacity>
            );
          })}

          {target === 'strava' ? (
            <TouchableOpacity
              onPress={() => setShowStravaBYO(true)}
              activeOpacity={0.7}
              className="mt-1"
            >
              <View
                className="flex-row items-center rounded-xl px-5 py-4 border"
                style={{
                  backgroundColor: colors.background.secondary,
                  borderColor: colors.border.default,
                }}
              >
                <View
                  className="w-10 h-10 rounded-xl items-center justify-center mr-4"
                  style={{ backgroundColor: BRAND_COLORS.strava.primary }}
                >
                  <Key size={20} color={PLATE_INK} />
                </View>
                <View className="flex-1">
                  <Text className="text-base font-semibold text-text-primary">
                    {t('app.useOwnStravaApp')}
                  </Text>
                  <Text className="text-xs mt-0.5 text-text-tertiary">
                    {t('app.stravaDevAppHint')}
                  </Text>
                </View>
              </View>
            </TouchableOpacity>
          ) : null}

          <Text className="text-xs text-text-tertiary text-center mt-2 px-4">
            {t('app.secureSessionBlurb')}
          </Text>
        </View>
      );
    }

    // Credentials form
    if (phase === 'credentials') {
      return (
        <View>
          <View className="flex-row items-center mb-5">
            <View
              className="w-8 h-8 rounded-lg items-center justify-center mr-3"
              style={{ backgroundColor: methodConfig.bgColor }}
            >
              {methodConfig.renderIcon(16)}
            </View>
            <Text className="text-sm text-text-secondary font-medium">
              {t(methodConfig.titleKey)}
            </Text>
          </View>

          {/* The exposure comes before the credentials: the account is told
              what connecting risks, and accepts it, before typing anything. */}
          {notice && (
            <ProviderNotice
              notice={notice}
              accepted={consentAccepted}
              onAcceptedChange={setConsentAccepted}
              testID="sciotte-tos-notice"
              consentTestID="sciotte-tos-consent"
            />
          )}

          <View className="mb-3">
            <Text className="text-xs text-text-tertiary mb-1.5 ml-1 font-medium">
              {t(usesUsername ? 'common.username' : 'common.email')}
            </Text>
            <TextInput
              className="bg-background-secondary rounded-xl px-4 py-3.5 text-base text-text-primary border border-border"
              placeholder={t(methodConfig.emailPlaceholderKey)}
              placeholderTextColor={colors.text.tertiary}
              value={email}
              onChangeText={setEmail}
              autoCapitalize="none"
              keyboardType={usesUsername ? 'default' : 'email-address'}
              autoComplete={usesUsername ? 'username' : 'email'}
              testID="sciotte-email"
            />
          </View>

          <View className="mb-4">
            <Text className="text-xs text-text-tertiary mb-1.5 ml-1 font-medium">{t('common.password')}</Text>
            <View className="flex-row items-center bg-background-secondary rounded-xl border border-border">
              <TextInput
                className="flex-1 px-4 py-3.5 text-base text-text-primary"
                placeholder={t('app.enterYourPassword')}
                placeholderTextColor={colors.text.tertiary}
                value={password}
                onChangeText={setPassword}
                secureTextEntry={!showPassword}
                autoComplete="password"
                testID="sciotte-password"
              />
              <TouchableOpacity className="px-4 py-3" onPress={() => setShowPassword(!showPassword)}>
                {showPassword
                  ? <EyeOff size={18} color={colors.text.tertiary} />
                  : <Eye size={18} color={colors.text.tertiary} />
                }
              </TouchableOpacity>
            </View>
          </View>

          {error && (
            <View className="flex-row items-center bg-error/10 rounded-lg px-3 py-2.5 mb-4">
              <AlertCircle size={16} color={colors.error} />
              <Text className="text-sm text-error ml-2 flex-1">{error}</Text>
            </View>
          )}

          <TouchableOpacity
            onPress={handleLogin}
            disabled={!canSubmit}
            accessibilityState={{ disabled: !canSubmit }}
            activeOpacity={0.8}
            testID="sciotte-login-submit"
          >
            {/* A disabled plate keeps its brand and fades, the way a disabled Button does. */}
            <LinearGradient
              colors={brandColor.gradient}
              start={{ x: 0, y: 0 }}
              end={{ x: 1, y: 0 }}
              style={{ borderRadius: 12, paddingVertical: 16, alignItems: 'center', opacity: canSubmit ? 1 : 0.5 }}
            >
              <Text className="text-base font-bold" style={{ color: PLATE_INK }}>
                {t('common.login')}
              </Text>
            </LinearGradient>
          </TouchableOpacity>

          {canGoBack && (
            <TouchableOpacity
              className="flex-row items-center justify-center mt-4 py-2"
              onPress={() => setPhase('choose')}
            >
              <ArrowLeft size={14} color={colors.text.tertiary} />
              <Text className="text-sm text-text-tertiary ml-1">{t('app.otherSignInMethods')}</Text>
            </TouchableOpacity>
          )}
        </View>
      );
    }

    // Loading/connecting
    if (phase === 'logging-in' || phase === 'waiting-approval') {
      return (
        <View className="items-center py-10">
          <View className="w-16 h-16 rounded-2xl items-center justify-center mb-5" style={{ backgroundColor: `${colors.tokens.primary}20` }}>
            <ActivityIndicator size="large" color={colors.tokens.primary} />
          </View>
          <Text className="text-base font-medium text-text-primary mb-1">{status}</Text>
          <Text className="text-sm text-text-tertiary text-center px-6">
            {phase === 'waiting-approval'
              ? t('app.openAuthenticatorApp')
              : t('app.connectingMoment')}
          </Text>
        </View>
      );
    }

    // 2FA choice
    if (phase === 'two-factor') {
      return (
        <View>
          <View className="items-center mb-5">
            <View className="w-12 h-12 rounded-2xl items-center justify-center mb-3" style={{ backgroundColor: `${colors.pierre.violet}20` }}>
              <Shield size={24} color={colors.pierre.violet} />
            </View>
            <Text className="text-base font-medium text-text-primary">{t('app.twoFactorAuth')}</Text>
            <Text className="text-sm text-text-tertiary mt-1">{t('app.chooseVerificationMethod')}</Text>
          </View>
          <View className="gap-2">
            {twoFactorOptions.map((option) => (
              <TouchableOpacity
                key={option.id}
                className="bg-background-secondary rounded-xl p-4 border border-border active:border-primary"
                onPress={() => handleSelect2FA(option.id)}
                disabled={isLoading}
                activeOpacity={0.7}
              >
                <Text className="text-base text-text-primary font-medium text-center">
                  {option.label}
                </Text>
              </TouchableOpacity>
            ))}
          </View>
        </View>
      );
    }

    // Number match
    if (phase === 'number-match') {
      return (
        <View className="items-center py-6">
          <View className="w-12 h-12 rounded-2xl items-center justify-center mb-4" style={{ backgroundColor: `${colors.pierre.violet}20` }}>
            <Shield size={24} color={colors.pierre.violet} />
          </View>
          <Text className="text-sm text-text-secondary mb-4">{t('app.tapThisNumber')}</Text>
          {matchNumber && (
            <View
              className="rounded-2xl px-12 py-6 mb-5"
              style={{ ...cardStyle, borderRadius: 20 }}
            >
              <Text className="text-5xl font-bold text-center text-text-primary">
                {matchNumber}
              </Text>
            </View>
          )}
          <ActivityIndicator size="small" color={colors.text.tertiary} />
          <Text className="text-xs text-text-tertiary mt-3">{t('app.waitingConfirmation')}</Text>
        </View>
      );
    }

    // OTP input
    if (phase === 'otp') {
      return (
        <View>
          <View className="items-center mb-5">
            <View className="w-12 h-12 rounded-2xl items-center justify-center mb-3" style={{ backgroundColor: `${colors.pierre.violet}20` }}>
              <Shield size={24} color={colors.pierre.violet} />
            </View>
            <Text className="text-base font-medium text-text-primary">{t('app.verificationCode')}</Text>
            <Text className="text-sm text-text-tertiary mt-1">{t('app.enterAuthenticatorCode')}</Text>
          </View>

          <TextInput
            className="bg-background-secondary rounded-xl px-4 py-4 mb-4 text-text-primary text-center text-2xl tracking-[8px] font-mono border border-border"
            placeholder="000000"
            placeholderTextColor={colors.text.tertiary}
            value={otpCode}
            onChangeText={setOtpCode}
            keyboardType="number-pad"
            maxLength={6}
            autoFocus
            testID="sciotte-otp"
          />

          <TouchableOpacity
            onPress={handleOtpSubmit}
            disabled={!otpCode || isLoading}
            activeOpacity={0.8}
          >
            <LinearGradient
              colors={brandColor.gradient}
              start={{ x: 0, y: 0 }}
              end={{ x: 1, y: 0 }}
              style={{ borderRadius: 12, paddingVertical: 16, alignItems: 'center', opacity: !otpCode || isLoading ? 0.5 : 1 }}
            >
              {isLoading ? (
                <ActivityIndicator size="small" color={PLATE_INK} />
              ) : (
                <Text className="text-base font-bold" style={{ color: PLATE_INK }}>{t('app.verify')}</Text>
              )}
            </LinearGradient>
          </TouchableOpacity>
        </View>
      );
    }

    // Success
    if (phase === 'success') {
      return (
        <View className="items-center py-10">
          <View className="w-16 h-16 rounded-full items-center justify-center mb-4" style={{ backgroundColor: `${colors.success}15` }}>
            <CheckCircle2 size={32} color={colors.success} />
          </View>
          <Text className="text-lg font-bold text-text-primary">{t('app.connectedBang')}</Text>
          <Text className="text-sm text-text-secondary mt-1">{platformName} is ready to sync</Text>
        </View>
      );
    }

    // Error
    if (phase === 'error') {
      return (
        <View className="items-center py-6">
          <View className="w-16 h-16 rounded-full items-center justify-center mb-4" style={{ backgroundColor: `${colors.error}15` }}>
            <AlertCircle size={32} color={colors.error} />
          </View>
          <Text className="text-base font-medium text-text-primary mb-1">{t('app.connectionFailed')}</Text>
          <Text className="text-sm text-error text-center mb-5 px-4">{error}</Text>
          <TouchableOpacity
            onPress={() => setPhase('credentials')}
            activeOpacity={0.8}
          >
            <LinearGradient
              colors={brandColor.gradient}
              start={{ x: 0, y: 0 }}
              end={{ x: 1, y: 0 }}
              style={{ borderRadius: 12, paddingHorizontal: 32, paddingVertical: 12 }}
            >
              <Text className="text-base font-semibold" style={{ color: PLATE_INK }}>{t('app.tryAgain')}</Text>
            </LinearGradient>
          </TouchableOpacity>
        </View>
      );
    }

    return null;
  };

  return (
    <>
    <Modal visible={visible} animationType="slide" transparent onRequestClose={onClose}>
      <KeyboardAvoidingView
        behavior={Platform.OS === 'ios' ? 'padding' : 'height'}
        className="flex-1"
      >
        {/* The backdrop closes the sheet on a tap, but it must not be one
            accessibility element: a touchable is accessible by default, and
            iOS then folds every field, checkbox and button inside the sheet
            into it, so neither VoiceOver nor a UI driver can reach them. The
            sheet's close button is the accessible way out. */}
        <TouchableOpacity
          className="flex-1 bg-scrim/60 justify-end"
          activeOpacity={1}
          onPress={onClose}
          accessible={false}
        >
          <View
            className="bg-background-primary rounded-t-3xl overflow-hidden"
            style={{ paddingBottom: 40 }}
            onStartShouldSetResponder={() => true}
          >
            {/* Accent gradient line */}
            <LinearGradient
              colors={brandColor.gradient}
              start={{ x: 0, y: 0 }}
              end={{ x: 1, y: 0 }}
              style={{ height: 3, width: '100%' }}
            />

            {/* Drag handle */}
            <View className="items-center pt-3 pb-1">
              <View className="w-10 h-1 rounded-full bg-border-default" />
            </View>

            {/* Header */}
            <View className="flex-row items-center justify-between px-6 py-3">
              <View className="flex-row items-center flex-1">
                <LinearGradient
                  colors={brandColor.gradient}
                  start={{ x: 0, y: 0 }}
                  end={{ x: 1, y: 1 }}
                  style={{ width: 44, height: 44, borderRadius: 12, alignItems: 'center', justifyContent: 'center', marginRight: 12 }}
                >
                  {preset.renderLogo(22)}
                </LinearGradient>
                <View className="flex-1">
                  <Text className="text-lg font-bold text-text-primary">
                    {platformName}
                  </Text>
                  {status && phase !== 'choose' && phase !== 'credentials' ? (
                    <Text className="text-xs text-text-tertiary">{status}</Text>
                  ) : (
                    <Text className="text-xs text-text-tertiary">{t('app.secureConnection')}</Text>
                  )}
                </View>
              </View>
              <TouchableOpacity
                className="w-8 h-8 rounded-full bg-background-secondary items-center justify-center"
                onPress={onClose}
              >
                <X size={16} color={colors.text.tertiary} />
              </TouchableOpacity>
            </View>

            {/* Content */}
            <ScrollView
              className="px-6 pt-2 pb-4"
              style={{ maxHeight: contentMaxHeight }}
              showsVerticalScrollIndicator={false}
              keyboardShouldPersistTaps="handled"
            >
              {renderContent()}
            </ScrollView>
          </View>
        </TouchableOpacity>
      </KeyboardAvoidingView>
    </Modal>

    <OAuthAppSetupModal
      visible={showStravaBYO}
      onClose={() => setShowStravaBYO(false)}
      onSaved={launchStravaOAuthFromBYO}
      provider="strava"
      displayName="Strava"
      devPortalUrl="https://www.strava.com/settings/api"
    />
    </>
  );
}
