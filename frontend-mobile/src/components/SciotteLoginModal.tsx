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
} from 'react-native';
import * as WebBrowser from 'expo-web-browser';
import * as Linking from 'expo-linking';
import { LinearGradient } from 'expo-linear-gradient';
import { Mail, ArrowLeft, Eye, EyeOff, Shield, CheckCircle2, AlertCircle, X, Key } from 'lucide-react-native';
import { glassCard, useThemeColors } from '../constants/theme';
import { oauthApi } from '../services/api';
import { getOAuthCallbackUrl } from '../utils/oauth';
import { StravaLogo, GarminLogo, GoogleLogo, AppleLogo } from './icons/BrandIcons';
import { OAuthAppSetupModal } from './OAuthAppSetupModal';
import { useTranslation } from '@pierre/i18n';
import { PROVIDER_BRAND } from '../constants/brands';

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
  target: 'strava' | 'garmin';
}

const BRAND_COLORS = {
  strava: { primary: '#FC4C02', gradient: ['#FC4C02', '#E34402'] as [string, string] },
  garmin: { primary: '#007CC3', gradient: ['#007CC3', '#005A8E'] as [string, string] },
  google: { primary: '#4285F4', gradient: ['#4285F4', '#3367D6'] as [string, string] },
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
  textColor: string;
  bgColor: string;
}

const METHOD_CONFIGS: Record<LoginMethod, MethodConfig> = {
  email: {
    titleKey: 'app.emailAndPassword',
    emailPlaceholderKey: 'app.emailAddress',
    brandName: '',
    renderIcon: (size: number) => <Mail size={size} color="#FFFFFF" />,
    brandGradient: BRAND_COLORS.strava.gradient,
    textColor: '#FFFFFF',
    bgColor: `${BRAND_COLORS.strava.primary}20`,
  },
  google: {
    titleKey: 'app.brandGoogle',
    emailPlaceholderKey: 'app.googleEmail',
    brandName: PROVIDER_BRAND.google,
    renderIcon: (size: number) => <GoogleLogo size={size} />,
    brandGradient: BRAND_COLORS.google.gradient,
    textColor: '#FFFFFF',
    bgColor: '#FFFFFF',
  },
  apple: {
    titleKey: 'app.brandApple',
    emailPlaceholderKey: 'app.appleIdEmail',
    brandName: PROVIDER_BRAND.apple,
    renderIcon: (size: number) => <AppleLogo size={size} color="#FFFFFF" />,
    brandGradient: BRAND_COLORS.apple.gradient,
    textColor: '#000000',
    bgColor: '#000000',
  },
};

export function SciotteLoginModal({
  visible,
  onClose,
  onConnected,
  target,
}: SciotteLoginModalProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
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

  const brandColor = target === 'garmin' ? BRAND_COLORS.garmin : BRAND_COLORS.strava;
  // Brand names, not copy: identical in every locale.
  const platformName = target === 'garmin' ? t('app.brandGarminConnect') : t('app.brandStrava');

  useEffect(() => {
    if (visible) {
      setPhase(target === 'garmin' ? 'credentials' : 'choose');
      setMethod('email');
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
      const message = err instanceof Error ? err.message : t('app.stravaOauthLaunchFailed');
      Alert.alert(t('app.connectionFailed'), message);
    }
  }, [onConnected, onClose, t]);

  const handleLogin = useCallback(async () => {
    if (!email || !password) return;

    setIsLoading(true);
    setError(null);
    setPhase('logging-in');
    setStatus(t('app.signingInTo', { provider: platformName }));

    try {
      const data = await oauthApi.sciotteLogin({ email, password, method, target });

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
      const message = err instanceof Error ? err.message : t('auth.loginFailed');
      setError(message);
      setPhase('error');
    } finally {
      setIsLoading(false);
    }
  }, [email, password, method, target, platformName, onClose, onConnected, t]);

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
      setError(err instanceof Error ? err.message : t('app.verificationFailed'));
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
      setError(err instanceof Error ? err.message : t('app.verificationFailed'));
      setPhase('error');
    } finally {
      setIsLoading(false);
    }
  }, [otpCode, onClose, onConnected, t]);

  const methodConfig = target === 'garmin'
    ? { ...METHOD_CONFIGS.email, title: t('app.garminAccount'), emailPlaceholder: t('app.garminEmail') }
    : METHOD_CONFIGS[method];

  const canGoBack = phase === 'credentials' && target === 'strava';

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
                  style={{ backgroundColor: `${BRAND_COLORS.strava.primary}20` }}
                >
                  <Key size={20} color={BRAND_COLORS.strava.primary} />
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

          <View className="mb-3">
            <Text className="text-xs text-text-tertiary mb-1.5 ml-1 font-medium uppercase tracking-wide">{t('common.email')}</Text>
            <TextInput
              className="bg-background-secondary rounded-xl px-4 py-3.5 text-base text-text-primary border border-border-default"
              placeholder={t(methodConfig.emailPlaceholderKey)}
              placeholderTextColor={colors.text.tertiary}
              value={email}
              onChangeText={setEmail}
              autoCapitalize="none"
              keyboardType="email-address"
              autoComplete="email"
              testID="sciotte-email"
            />
          </View>

          <View className="mb-4">
            <Text className="text-xs text-text-tertiary mb-1.5 ml-1 font-medium uppercase tracking-wide">{t('common.password')}</Text>
            <View className="flex-row items-center bg-background-secondary rounded-xl border border-border-default">
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
            disabled={!email || !password}
            activeOpacity={0.8}
          >
            <LinearGradient
              colors={(!email || !password) ? ['#333', '#333'] : brandColor.gradient}
              start={{ x: 0, y: 0 }}
              end={{ x: 1, y: 0 }}
              style={{ borderRadius: 12, paddingVertical: 16, alignItems: 'center' }}
            >
              <Text className="text-base font-bold text-on-surface">
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
          <View className="w-16 h-16 rounded-2xl items-center justify-center mb-5" style={{ backgroundColor: `${brandColor.primary}15` }}>
            <ActivityIndicator size="large" color={brandColor.primary} />
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
                className="bg-background-secondary rounded-xl p-4 border border-border-default active:border-primary"
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
              style={{ ...glassCard, borderRadius: 20 }}
            >
              <Text className="text-5xl font-bold text-center" style={{ color: brandColor.primary }}>
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
            className="bg-background-secondary rounded-xl px-4 py-4 mb-4 text-text-primary text-center text-2xl tracking-[8px] font-mono border border-border-default"
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
              colors={(!otpCode || isLoading) ? ['#333', '#333'] : brandColor.gradient}
              start={{ x: 0, y: 0 }}
              end={{ x: 1, y: 0 }}
              style={{ borderRadius: 12, paddingVertical: 16, alignItems: 'center' }}
            >
              {isLoading ? (
                <ActivityIndicator size="small" color="#FFFFFF" />
              ) : (
                <Text className="text-base font-bold text-on-surface">{t('app.verify')}</Text>
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
              <Text className="text-base font-semibold text-on-surface">{t('app.tryAgain')}</Text>
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
        <TouchableOpacity
          className="flex-1 bg-black/60 justify-end"
          activeOpacity={1}
          onPress={onClose}
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
                  {target === 'garmin'
                    ? <GarminLogo size={22} color="#FFFFFF" />
                    : <StravaLogo size={22} color="#FFFFFF" />
                  }
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
              style={{ maxHeight: 420 }}
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
