// ABOUTME: Login screen with email/password and Google Sign-In authentication
// ABOUTME: Boreal Editorial hero backdrop with floating glass form card

import React, { useState, useEffect } from 'react';
import {
  View,
  Text,
  ScrollView,
  TouchableOpacity,
  Alert,
  Image,
  ActivityIndicator,
  type ImageStyle,
  type ViewStyle,
  type TextStyle,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { LinearGradient } from 'expo-linear-gradient';
import { StatusBar } from 'expo-status-bar';
import { useAuth } from '../../contexts/AuthContext';
import { Button, Input } from '../../components/ui';
import { PROVIDER_COLORS, spacing, glassCard, buttonGlow, BOREAL_LIGHT } from '../../constants/theme';
import {
  isFirebaseEnabled,
  useGoogleAuth,
  signInWithGoogleResponse,
} from '../../firebase';
import { AntDesign } from '@expo/vector-icons';
import { useRouter } from 'expo-router';

export function LoginScreen() {
  const router = useRouter();
  const { login, loginWithFirebase } = useAuth();
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [isLoading, setIsLoading] = useState(false);
  const [isGoogleLoading, setIsGoogleLoading] = useState(false);
  const [errors, setErrors] = useState<{ email?: string; password?: string }>({});

  // Google OAuth hook - always call unconditionally (React Rules of Hooks)
  // Hook returns null values when Firebase is not enabled
  const googleAuth = useGoogleAuth();

  // Handle Google OAuth response
  useEffect(() => {
    if (!googleAuth?.response) return;

    const handleGoogleResponse = async () => {
      if (googleAuth.response?.type === 'success') {
        setIsGoogleLoading(true);
        try {
          const result = await signInWithGoogleResponse(googleAuth.response);
          if (result) {
            await loginWithFirebase(result.idToken);
            // Navigation handled by auth state change
          }
        } catch (error) {
          let message = 'Google sign-in failed. Please try again.';
          if (error instanceof Error) {
            message = error.message;
          }
          Alert.alert('Sign In Failed', message);
        } finally {
          setIsGoogleLoading(false);
        }
      } else if (googleAuth.response?.type === 'error') {
        Alert.alert('Sign In Error', 'Google sign-in was cancelled or failed.');
        setIsGoogleLoading(false);
      } else {
        // Handle 'dismiss', 'cancel', 'locked' - reset loading state
        setIsGoogleLoading(false);
      }
    };

    handleGoogleResponse();
  }, [googleAuth.response, loginWithFirebase]);

  const validateForm = () => {
    const newErrors: { email?: string; password?: string } = {};

    if (!email.trim()) {
      newErrors.email = 'Email is required';
    } else if (!/\S+@\S+\.\S+/.test(email)) {
      newErrors.email = 'Please enter a valid email';
    }

    if (!password) {
      newErrors.password = 'Password is required';
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
        } else if (error.message.includes('Network')) {
          message = 'Network error. Please check your connection.';
        } else {
          message = error.message;
        }
      }
      Alert.alert('Login Failed', message);
    } finally {
      setIsLoading(false);
    }
  };

  const handleGoogleSignIn = async () => {
    if (!googleAuth?.promptAsync) {
      Alert.alert('Not Available', 'Google Sign-In is not configured.');
      return;
    }

    setIsGoogleLoading(true);
    try {
      await googleAuth.promptAsync();
      // Response is handled in useEffect above
    } catch (error) {
      let message = 'Failed to start Google sign-in.';
      if (error instanceof Error) {
        message = error.message;
      }
      Alert.alert('Error', message);
      setIsGoogleLoading(false);
    }
  };

  // Editorial hero wordmark — "DRAVR" in Space Grotesk with brand letter-spacing
  const wordmarkStyle: TextStyle = {
    fontFamily: 'SpaceGrotesk_SemiBold',
    fontSize: 28,
    letterSpacing: 4.2, // ≈ 0.15em on 28px
    color: '#a3d0be', // primaryFixedDim — reads on deep-forest backdrop
  };

  // Editorial over-line ("The Technical Naturalist") — tiny tracked label
  const kickerStyle: TextStyle = {
    fontFamily: 'Inter_Medium',
    fontSize: 11,
    letterSpacing: 2,
    textTransform: 'uppercase',
    color: '#a3d0be',
    opacity: 0.85,
  };

  // Hero headline — editorial display typography
  const heroHeadlineStyle: TextStyle = {
    fontFamily: 'SpaceGrotesk_SemiBold',
    fontSize: 28,
    lineHeight: 34,
    color: '#ffffff',
  };

  const heroLeadStyle: TextStyle = {
    fontFamily: 'PlusJakartaSans',
    fontSize: 14,
    lineHeight: 22,
    color: '#a3d0be',
  };

  // Logo style (pixel-specific dimensions)
  const logoStyle: ImageStyle = { width: 48, height: 48 };

  // Elevated card surface — sits above the forest backdrop
  const cardStyle: ViewStyle = {
    ...glassCard,
    backgroundColor: BOREAL_LIGHT.surfaceContainerLowest, // pure white
    borderRadius: 20,
    overflow: 'hidden',
    shadowOpacity: 0.18,
    shadowRadius: 32,
  };

  // Button with ambient shadow
  const glowButtonStyle: ViewStyle = {
    ...buttonGlow,
    marginTop: spacing.md,
  };

  return (
    <View className="flex-1" testID="login-screen" style={{ backgroundColor: BOREAL_LIGHT.primary }}>
      {/* Full-bleed Boreal hero backdrop — 145° primary → primary-container */}
      <LinearGradient
        colors={['#00241a', '#0d3b2e']}
        start={{ x: 0, y: 0 }}
        end={{ x: 1, y: 1 }}
        style={{ position: 'absolute', top: 0, left: 0, right: 0, bottom: 0 }}
      />
      <StatusBar style="light" />
      <SafeAreaView className="flex-1" edges={['top', 'bottom']}>
        <ScrollView
          contentContainerStyle={{ flexGrow: 1, paddingHorizontal: spacing.lg, paddingVertical: spacing.xl }}
          keyboardShouldPersistTaps="handled"
          automaticallyAdjustKeyboardInsets
          testID="login-scroll-view"
        >
          {/* Editorial hero band — brand moment at the top */}
          <View style={{ marginTop: spacing.md, marginBottom: spacing.xl }}>
            <View className="flex-row items-center" style={{ gap: spacing.sm, marginBottom: spacing.lg }}>
              <Image
                source={require('../../../assets/dravr-logo.png')}
                style={logoStyle}
                resizeMode="contain"
              />
              <Text style={wordmarkStyle}>DRAVR</Text>
            </View>
            <Text style={kickerStyle}>The Technical Naturalist</Text>
            <Text style={[heroHeadlineStyle, { marginTop: spacing.sm, marginBottom: spacing.sm }]}>
              Fitness intelligence,{'\n'}rendered in ink.
            </Text>
            <Text style={heroLeadStyle}>
              Every session, meal, and hour of recovery — a single, legible story.
            </Text>
          </View>

          {/* Floating form card — white surface over the forest hero. Every
              element inside this region uses BOREAL_LIGHT directly because
              the card is brand-fixed and must read correctly even when the
              user's global preference is dark. */}
          <View style={cardStyle}>
            <View className="px-6 py-7">
              <View className="mb-5">
                <Text
                  style={{
                    fontFamily: 'SpaceGrotesk_SemiBold',
                    fontSize: 24,
                    marginBottom: 4,
                    color: BOREAL_LIGHT.onSurface,
                  }}
                >
                  Sign in
                </Text>
                <Text style={{ fontSize: 14, color: BOREAL_LIGHT.onSurfaceVariant }}>
                  Welcome back. Enter your credentials to continue.
                </Text>
              </View>

              {/* Login Form */}
              <View className="mb-2">
                <Input
                  label="Email"
                  placeholder="you@example.com"
                  value={email}
                  onChangeText={setEmail}
                  keyboardType="email-address"
                  autoCapitalize="none"
                  autoCorrect={false}
                  error={errors.email}
                  surface="light"
                  testID="email-input"
                />

                <Input
                  label="Password"
                  placeholder="Enter your password"
                  value={password}
                  onChangeText={setPassword}
                  secureTextEntry
                  showPasswordToggle
                  returnKeyType="go"
                  onSubmitEditing={handleLogin}
                  error={errors.password}
                  surface="light"
                  testID="password-input"
                />

                <TouchableOpacity
                  onPress={() => router.push('/(auth)/forgot-password')}
                  className="self-end mb-2"
                  testID="forgot-password-link"
                >
                  <Text style={{ fontSize: 12, color: BOREAL_LIGHT.outline }}>
                    Forgot password?
                  </Text>
                </TouchableOpacity>

                <Button
                  title="Sign In"
                  onPress={handleLogin}
                  loading={isLoading}
                  fullWidth
                  surface="light"
                  style={glowButtonStyle}
                  testID="login-button"
                />

                {/* Google Sign-In - only show when Firebase is configured */}
                {isFirebaseEnabled() && (
                  <>
                    <View className="flex-row items-center my-5">
                      <View
                        style={{ flex: 1, height: 1, backgroundColor: BOREAL_LIGHT.outlineVariant }}
                      />
                      <Text
                        className="px-3"
                        style={{ fontSize: 13, color: BOREAL_LIGHT.onSurfaceVariant }}
                      >
                        or continue with
                      </Text>
                      <View
                        style={{ flex: 1, height: 1, backgroundColor: BOREAL_LIGHT.outlineVariant }}
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
                        borderColor: 'rgba(26, 28, 27, 0.12)',
                        backgroundColor: BOREAL_LIGHT.surfaceContainerLowest,
                      }}
                    >
                      {isGoogleLoading ? (
                        <ActivityIndicator size="small" color={BOREAL_LIGHT.onSurface} />
                      ) : (
                        <AntDesign name="google" size={20} color={PROVIDER_COLORS.google} />
                      )}
                      <Text
                        style={{
                          fontSize: 15,
                          fontWeight: '500',
                          color: BOREAL_LIGHT.onSurface,
                        }}
                      >
                        {isGoogleLoading ? 'Signing in…' : 'Continue with Google'}
                      </Text>
                    </TouchableOpacity>
                  </>
                )}
              </View>

              {/* Register Link */}
              <View className="flex-row justify-center items-center gap-1 pt-4">
                <Text style={{ fontSize: 13, color: BOREAL_LIGHT.onSurfaceVariant }}>
                  Don&apos;t have an account?
                </Text>
                <TouchableOpacity onPress={() => router.push('/(auth)/register')}>
                  <Text
                    style={{ fontSize: 13, fontWeight: '600', color: BOREAL_LIGHT.primary }}
                  >
                    Create one
                  </Text>
                </TouchableOpacity>
              </View>
            </View>
          </View>

          {/* Footer band — fitness pillars */}
          <View
            className="flex-row items-center justify-center"
            style={{ gap: spacing.md, marginTop: spacing.xl, opacity: 0.7 }}
          >
            {['Activity', 'Nutrition', 'Recovery', 'Mobility'].map((pillar, i) => (
              <React.Fragment key={pillar}>
                {i > 0 && (
                  <Text style={{ color: '#a3d0be', opacity: 0.5 }} aria-hidden>
                    ·
                  </Text>
                )}
                <Text
                  style={{
                    fontFamily: 'Inter_Medium',
                    fontSize: 10,
                    letterSpacing: 1.2,
                    textTransform: 'uppercase',
                    color: '#a3d0be',
                  }}
                >
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
