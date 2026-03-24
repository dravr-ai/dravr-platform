// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Login modal for sciotte provider — collects credentials, Pierre runs Chrome in-process
// ABOUTME: Supports Google/Apple/email login methods with 2FA (OTP and phone tap)

import { useCallback, useEffect, useState } from 'react';
import { pierreApi } from '../services/api';

// Sciotte drives a headless browser through OAuth — give it plenty of time
// 20-minute timeout — sciotte drives a headless browser through OAuth + 2FA
const SCIOTTE_TIMEOUT_MS = Number(import.meta.env.VITE_SCIOTTE_TIMEOUT_MS) || 1_200_000;

type LoginPhase = 'choose' | 'credentials' | 'logging-in' | 'two-factor' | 'waiting-approval' | 'number-match' | 'otp' | 'success' | 'error';

interface TwoFactorOption {
  id: string;
  label: string;
}
type LoginMethod = 'email' | 'google' | 'apple';

interface SciotteLoginModalProps {
  isOpen: boolean;
  onClose: () => void;
  onConnected: () => void;
  /** Target platform: "strava" or "garmin" */
  target?: 'strava' | 'garmin';
}

const METHOD_LABELS: Record<LoginMethod, { title: string; emailPlaceholder: string }> = {
  email: { title: 'Strava Account', emailPlaceholder: 'Strava email' },
  google: { title: 'Google Account', emailPlaceholder: 'Google email' },
  apple: { title: 'Apple Account', emailPlaceholder: 'Apple ID email' },
};

export default function SciotteLoginModal({
  isOpen,
  onClose,
  onConnected,
  target = 'strava',
}: SciotteLoginModalProps) {
  const [phase, setPhase] = useState<LoginPhase>('choose');
  const [method, setMethod] = useState<LoginMethod>('email');
  const [status, setStatus] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [otpCode, setOtpCode] = useState('');
  const [twoFactorOptions, setTwoFactorOptions] = useState<TwoFactorOption[]>([]);
  const [showPassword, setShowPassword] = useState(false);
  const [matchNumber, setMatchNumber] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(false);

  useEffect(() => {
    if (isOpen) {
      // Garmin uses direct email/password — skip the choose phase
      setPhase(target === 'garmin' ? 'credentials' : 'choose');
      setMethod('email');
      setStatus('');
      setError(null);
      setEmail('');
      setPassword('');
      setOtpCode('');
      setIsLoading(false);
    }
  }, [isOpen, target]);

  const selectMethod = (m: LoginMethod) => {
    setMethod(m);
    setPhase('credentials');
    setError(null);
  };

  // Email/password login — in-process headless Chrome
  const handleEmailLogin = useCallback(
    async (e: React.FormEvent) => {
      e.preventDefault();
      if (!email || !password) return;

      setIsLoading(true);
      setError(null);
      setPhase('logging-in');
      setStatus('Connecting to Strava...');

      try {
        const { data } = await pierreApi.axios.post('/api/providers/sciotte/login', {
          email,
          password,
          method,
          target,
        }, { timeout: SCIOTTE_TIMEOUT_MS });

        if (data.status === 'connected') {
          setPhase('success');
          setStatus('Connected!');
          onConnected();
          setTimeout(onClose, 1500);
        } else if (data.status === 'two_factor_choice') {
          setTwoFactorOptions(data.options || []);
          setPhase('two-factor');
          setStatus('Choose verification method');
        } else if (data.status === 'otp_required') {
          setPhase('otp');
          setStatus('Enter verification code');
          setOtpCode('');
        } else if (data.status === 'number_match') {
          setMatchNumber(data.number);
          setPhase('number-match');
          setStatus('Tap the matching number on your phone');
        } else {
          setError(data.error || 'Login failed');
          setPhase('error');
        }
      } catch (err) {
        const message =
          (err as { response?: { data?: { error?: string } } })?.response?.data?.error ||
          `Login failed: ${err}`;
        setError(message);
        setPhase('error');
      } finally {
        setIsLoading(false);
      }
    },
    [email, password, onClose, onConnected]
  );

  // 2FA option selection
  const handleSelectTwoFactor = useCallback(
    async (optionId: string) => {
      setIsLoading(true);
      // Don't change phase for poll (number-match auto-poll) or app (waiting-approval)
      if (optionId === 'app') setPhase('waiting-approval');
      else if (optionId !== 'poll') setPhase('logging-in');
      setStatus(optionId === 'app' ? 'Check your phone and tap Yes...' : 'Loading...');

      try {
        const { data } = await pierreApi.axios.post('/api/providers/sciotte/select-2fa', {
          option_id: optionId,
        }, { timeout: SCIOTTE_TIMEOUT_MS });

        if (data.status === 'connected') {
          setPhase('success');
          setStatus('Connected!');
          onConnected();
          setTimeout(onClose, 1500);
        } else if (data.status === 'otp_required') {
          setPhase('otp');
          setStatus('Enter verification code');
          setOtpCode('');
        } else if (data.status === 'number_match') {
          setMatchNumber(data.number);
          setPhase('number-match');
          setStatus('Tap the matching number on your phone');
        } else {
          setError(data.error || 'Verification failed');
          setPhase('error');
        }
      } catch (err) {
        const message =
          (err as { response?: { data?: { error?: string } } })?.response?.data?.error ||
          `Verification failed: ${err}`;
        setError(message);
        setPhase('error');
      } finally {
        setIsLoading(false);
      }
    },
    [onClose, onConnected]
  );

  // Auto-poll once when number-match phase is reached — the phone notification
  // arrives before the UI shows the number, so poll immediately
  const [pollingStarted, setPollingStarted] = useState(false);
  useEffect(() => {
    if (phase === 'number-match' && matchNumber && !pollingStarted) {
      setPollingStarted(true);
      handleSelectTwoFactor('poll');
    }
    if (phase !== 'number-match') {
      setPollingStarted(false);
    }
  }, [phase, matchNumber]); // eslint-disable-line react-hooks/exhaustive-deps

  // OTP submission
  const handleOtpSubmit = useCallback(
    async (e: React.FormEvent) => {
      e.preventDefault();
      if (!otpCode) return;

      setIsLoading(true);
      setPhase('logging-in');
      setStatus('Verifying code...');

      try {
        const { data } = await pierreApi.axios.post('/api/providers/sciotte/submit-otp', {
          code: otpCode,
        }, { timeout: SCIOTTE_TIMEOUT_MS });

        if (data.status === 'connected') {
          setPhase('success');
          setStatus('Connected!');
          onConnected();
          setTimeout(onClose, 1500);
        } else if (data.status === 'otp_required') {
          setPhase('otp');
          setOtpCode('');
        } else {
          setError(data.error || 'Verification failed');
          setPhase('error');
        }
      } catch (err) {
        setError(`Verification failed: ${err}`);
        setPhase('error');
      } finally {
        setIsLoading(false);
      }
    },
    [otpCode, onClose, onConnected]
  );


  if (!isOpen) return null;

  const labels = target === 'garmin' && method === 'email'
    ? { title: 'Garmin Account', emailPlaceholder: 'Garmin email' }
    : METHOD_LABELS[method];

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm">
      <div className="bg-pierre-gray-900 rounded-2xl border border-white/10 shadow-2xl max-w-md w-full mx-4 overflow-hidden">
        {/* Header */}
        <div className="flex items-center justify-between px-6 py-4 border-b border-white/10">
          <div className="flex items-center gap-3">
            <div className="w-8 h-8 rounded-lg bg-gradient-to-br from-orange-500 to-orange-600 flex items-center justify-center flex-shrink-0">
              <svg className="w-4 h-4 text-white" viewBox="0 0 24 24" fill="currentColor">
                <path d="M15.387 17.944l-2.089-4.116h-3.065L15.387 24l5.15-10.172h-3.066m-7.008-5.599l2.836 5.598h4.172L10.463 0l-7 13.828h4.169" />
              </svg>
            </div>
            <div>
              <h2 className="text-lg font-semibold text-white">Connect to {target === 'garmin' ? 'Garmin' : 'Strava'}</h2>
              {status && <p className="text-xs text-white/50">{status}</p>}
            </div>
          </div>
          <button
            onClick={onClose}
            className="p-2 rounded-lg hover:bg-white/10 text-white/60 hover:text-white transition-colors"
            title="Close"
            aria-label="Close login modal"
          >
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        <div className="p-6">
          {/* Phase: Choose */}
          {phase === 'choose' && (
            <div className="space-y-3">
              <button
                onClick={() => selectMethod('google')}
                className="w-full flex items-center justify-center gap-3 px-4 py-3 bg-white/10 border border-white/20 rounded-lg hover:bg-white/20 hover:border-white/30 transition-all text-white font-medium"
              >
                <svg className="w-5 h-5 flex-shrink-0" viewBox="0 0 24 24">
                  <path d="M22.56 12.25c0-.78-.07-1.53-.2-2.25H12v4.26h5.92a5.06 5.06 0 01-2.2 3.32v2.77h3.57c2.08-1.92 3.28-4.74 3.28-8.1z" fill="#4285F4" />
                  <path d="M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84C3.99 20.53 7.7 23 12 23z" fill="#34A853" />
                  <path d="M5.84 14.09c-.22-.66-.35-1.36-.35-2.09s.13-1.43.35-2.09V7.07H2.18C1.43 8.55 1 10.22 1 12s.43 3.45 1.18 4.93l2.85-2.22.81-.62z" fill="#FBBC05" />
                  <path d="M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15C17.45 2.09 14.97 1 12 1 7.7 1 3.99 3.47 2.18 7.07l3.66 2.84c.87-2.6 3.3-4.53 6.16-4.53z" fill="#EA4335" />
                </svg>
                Continue with Google
              </button>

              <button
                onClick={() => selectMethod('apple')}
                className="w-full flex items-center justify-center gap-3 px-4 py-3 bg-white/10 border border-white/20 rounded-lg hover:bg-white/20 hover:border-white/30 transition-all text-white font-medium"
              >
                <svg className="w-5 h-5 flex-shrink-0" viewBox="0 0 24 24" fill="currentColor">
                  <path d="M17.05 20.28c-.98.95-2.05.88-3.08.4-1.09-.5-2.08-.48-3.24 0-1.44.62-2.2.44-3.06-.4C2.79 15.25 3.51 7.59 9.05 7.31c1.35.07 2.29.74 3.08.8 1.18-.24 2.31-.93 3.57-.84 1.51.12 2.65.72 3.4 1.8-3.12 1.87-2.38 5.98.48 7.13-.57 1.5-1.31 2.99-2.54 4.09zM12.03 7.25c-.15-2.23 1.66-4.07 3.74-4.25.29 2.58-2.34 4.5-3.74 4.25z" />
                </svg>
                Continue with Apple
              </button>

              <div className="relative my-4">
                <div className="absolute inset-0 flex items-center"><div className="w-full border-t border-white/10" /></div>
                <div className="relative flex justify-center text-xs">
                  <span className="px-3 bg-pierre-gray-900 text-white/50">Or</span>
                </div>
              </div>

              <button
                onClick={() => { setMethod('email'); setPhase('credentials'); }}
                className="w-full flex items-center justify-center gap-3 px-4 py-3 bg-white/5 border border-white/10 rounded-lg hover:bg-white/10 hover:border-white/20 transition-all text-white/80 font-medium"
              >
                <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M3 8l7.89 5.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z" />
                </svg>
                Log in with Strava email
              </button>

              <p className="text-xs text-white/30 text-center mt-3">
                Credentials go directly to the provider — Pierre never stores your password
              </p>
            </div>
          )}

          {/* Phase: Email credentials */}
          {phase === 'credentials' && (
            <div>
              <button onClick={() => target === 'garmin' ? onClose() : setPhase('choose')} className="flex items-center gap-1 text-sm text-white/50 hover:text-white/80 transition-colors mb-4">
                <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 19l-7-7 7-7" /></svg>
                Back
              </button>
              <h3 className="text-white font-medium mb-4">{labels.title}</h3>
              <form onSubmit={handleEmailLogin} className="space-y-4">
                <div>
                  <label htmlFor="sciotte-email" className="block text-sm text-white/60 mb-1.5">Email</label>
                  <input
                    id="sciotte-email"
                    type="email"
                    placeholder={labels.emailPlaceholder}
                    value={email}
                    onChange={(e) => setEmail(e.target.value)}
                    className="input-glass w-full"
                    required
                    autoComplete="email"
                    autoFocus
                    name="email"
                  />
                </div>
                <div>
                  <label htmlFor="sciotte-password" className="block text-sm text-white/60 mb-1.5">Password</label>
                  <div className="relative">
                    <input
                      id="sciotte-password"
                      type={showPassword ? 'text' : 'password'}
                      placeholder="Password"
                      value={password}
                      onChange={(e) => setPassword(e.target.value)}
                      className="input-glass w-full pr-10"
                      required
                      autoComplete="current-password"
                      name="password"
                    />
                    <button
                      type="button"
                      onClick={() => setShowPassword(!showPassword)}
                      className="absolute right-3 top-1/2 -translate-y-1/2 text-white/40 hover:text-white/70 transition-colors"
                      title={showPassword ? 'Hide password' : 'Show password'}
                      aria-label={showPassword ? 'Hide password' : 'Show password'}
                    >
                      {showPassword ? (
                        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13.875 18.825A10.05 10.05 0 0112 19c-4.478 0-8.268-2.943-9.543-7a9.97 9.97 0 011.563-3.029m5.858.908a3 3 0 114.243 4.243M9.878 9.878l4.242 4.242M9.88 9.88l-3.29-3.29m7.532 7.532l3.29 3.29M3 3l3.59 3.59m0 0A9.953 9.953 0 0112 5c4.478 0 8.268 2.943 9.543 7a10.025 10.025 0 01-4.132 5.411m0 0L21 21" />
                        </svg>
                      ) : (
                        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
                          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z" />
                        </svg>
                      )}
                    </button>
                  </div>
                </div>
                <button type="submit" disabled={isLoading || !email || !password} className="w-full py-3 bg-gradient-to-r from-pierre-nutrition to-orange-600 rounded-lg text-white font-medium hover:shadow-lg hover:shadow-pierre-nutrition/40 hover:-translate-y-0.5 transition-all disabled:opacity-50 disabled:cursor-not-allowed disabled:transform-none">
                  {isLoading ? 'Logging in...' : 'Log In'}
                </button>
              </form>
            </div>
          )}

          {/* Phase: Logging in */}
          {phase === 'logging-in' && (
            <div className="flex items-center justify-center py-12">
              <div className="text-center">
                <div className="pierre-spinner w-12 h-12 mx-auto mb-4 border-2 border-white/20 border-t-pierre-nutrition" />
                <p className="text-white/60 text-sm">{status}</p>
                <p className="text-white/30 text-xs mt-2">This may take up to 30 seconds</p>
              </div>
            </div>
          )}

          {/* Phase: Two-factor choice */}
          {phase === 'two-factor' && (
            <div>
              <div className="text-center mb-4">
                <div className="w-12 h-12 mx-auto mb-3 rounded-full bg-amber-500/10 flex items-center justify-center">
                  <svg className="w-6 h-6 text-amber-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z" />
                  </svg>
                </div>
                <p className="text-white font-medium">2-Step Verification</p>
                <p className="text-white/50 text-sm mt-1">Choose how to verify your identity</p>
              </div>
              <div className="space-y-3">
                {twoFactorOptions.map((option) => (
                  <button
                    key={option.id}
                    onClick={() => handleSelectTwoFactor(option.id)}
                    disabled={isLoading}
                    className="w-full flex items-center gap-3 px-4 py-3 bg-white/10 border border-white/20 rounded-lg hover:bg-white/20 hover:border-white/30 transition-all text-white text-left disabled:opacity-50"
                  >
                    <span className="text-sm">{option.label}</span>
                  </button>
                ))}
              </div>
            </div>
          )}

          {/* Phase: Waiting for phone approval */}
          {phase === 'waiting-approval' && (
            <div className="flex items-center justify-center py-12">
              <div className="text-center">
                <div className="pierre-spinner w-12 h-12 mx-auto mb-4 border-2 border-white/20 border-t-amber-500" />
                <p className="text-white font-medium">Check your phone</p>
                <p className="text-white/50 text-sm mt-1">Tap Yes on the notification</p>
              </div>
            </div>
          )}

          {/* Phase: Number match challenge — auto-polls for approval */}
          {phase === 'number-match' && matchNumber && (
            <div className="flex items-center justify-center py-12">
              <div className="text-center">
                <div className="w-24 h-24 mx-auto mb-4 rounded-2xl bg-gradient-to-br from-blue-500/20 to-blue-600/20 border-2 border-blue-500/40 flex items-center justify-center">
                  <span className="text-5xl font-bold text-blue-400">{matchNumber}</span>
                </div>
                <p className="text-white font-medium mb-1">Tap this number on your phone</p>
                <p className="text-white/50 text-sm mb-4">Check the Google notification on your device</p>
                <div className="pierre-spinner w-8 h-8 mx-auto border-2 border-white/20 border-t-blue-500" />
                <p className="text-white/30 text-xs mt-3">Waiting for you to tap...</p>
              </div>
            </div>
          )}

          {/* Phase: OTP */}
          {phase === 'otp' && (
            <div>
              <div className="text-center mb-4">
                <div className="w-12 h-12 mx-auto mb-3 rounded-full bg-amber-500/10 flex items-center justify-center">
                  <svg className="w-6 h-6 text-amber-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z" />
                  </svg>
                </div>
                <p className="text-white font-medium">Verification Required</p>
                <p className="text-white/50 text-sm mt-1">Enter the code sent to your device</p>
              </div>
              <form onSubmit={handleOtpSubmit} className="space-y-4">
                <input type="text" placeholder="Verification code" value={otpCode} onChange={(e) => setOtpCode(e.target.value)} className="input-glass w-full text-center text-lg tracking-widest" required autoFocus autoComplete="one-time-code" inputMode="numeric" />
                <button type="submit" disabled={isLoading || !otpCode} className="w-full py-3 bg-gradient-to-r from-pierre-nutrition to-orange-600 rounded-lg text-white font-medium hover:shadow-lg hover:shadow-pierre-nutrition/40 hover:-translate-y-0.5 transition-all disabled:opacity-50 disabled:cursor-not-allowed disabled:transform-none">
                  Verify
                </button>
              </form>
            </div>
          )}

          {/* Phase: Success */}
          {phase === 'success' && (
            <div className="flex items-center justify-center py-12">
              <div className="text-center">
                <div className="w-16 h-16 mx-auto mb-4 rounded-full bg-pierre-activity/10 flex items-center justify-center">
                  <svg className="w-8 h-8 text-pierre-activity" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" /></svg>
                </div>
                <p className="text-pierre-activity text-lg font-medium">Connected!</p>
                <p className="text-white/50 text-sm mt-1">Your Strava data is now available</p>
              </div>
            </div>
          )}

          {/* Phase: Error */}
          {phase === 'error' && (
            <div className="flex items-center justify-center py-12">
              <div className="text-center">
                <div className="w-16 h-16 mx-auto mb-4 rounded-full bg-red-500/10 flex items-center justify-center">
                  <svg className="w-8 h-8 text-red-400" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-2.5L13.732 4c-.77-.833-1.964-.833-2.732 0L4.082 16.5c-.77.833.192 2.5 1.732 2.5z" /></svg>
                </div>
                <p className="text-red-400 text-lg font-medium mb-2">Login Failed</p>
                <p className="text-white/50 text-sm max-w-sm mb-4">{error}</p>
                <button onClick={() => { setPhase(target === 'garmin' ? 'credentials' : 'choose'); setError(null); }} className="px-4 py-2 bg-white/10 border border-white/20 hover:bg-white/20 hover:border-white/30 rounded-lg text-white text-sm transition-all">
                  Try Again
                </button>
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
