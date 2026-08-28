// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Login modal for sciotte provider — collects credentials, Pierre runs Chrome in-process
// ABOUTME: Supports Google/Apple/email login methods with 2FA (OTP and phone tap)

import { useCallback, useEffect, useId, useState } from 'react';
import { oauthApi } from '../services/api';
import OAuthAppSetupModal from './OAuthAppSetupModal';
import { formatTimeout } from './sciotteLoginCopy';
import { useTranslation } from '@pierre/i18n';
import { describeApiError } from '@pierre/ui-logic';
import { useOnlineStatus } from '../hooks/useOnlineStatus';
import { useDialog } from '../hooks/useDialog';
import { RevealButton } from './ui';

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
  /**
   * Fired after the official OAuth popup is launched (Strava BYO-OAuth path).
   * Lets the parent show an "awaiting consent" state with timeout/cancel.
   * Connection success is still observed at the App level via the OAuth
   * callback URL — this is purely UI bookkeeping for the in-flight window.
   */
  onOAuthLaunched?: (provider: string) => void;
  /** Target platform: "strava" or "garmin" */
  target?: 'strava' | 'garmin';
}

// AppError serialises as { code, message, ... }; legacy/in-band errors sometimes
// expose { error }. Prefer message (current shape) then error, then the axios
// error itself as a last resort.
const METHOD_LABELS: Record<LoginMethod, { titleKey: string; emailPlaceholderKey: string }> = {
  email: { titleKey: 'shell.sciotteStravaAccount', emailPlaceholderKey: 'shell.stravaEmail' },
  google: { titleKey: 'shell.sciotteGoogleAccount', emailPlaceholderKey: 'shell.googleEmail' },
  apple: { titleKey: 'shell.sciotteAppleAccount', emailPlaceholderKey: 'shell.appleEmail' },
};

export default function SciotteLoginModal({
  isOpen,
  onClose,
  onConnected,
  onOAuthLaunched,
  target = 'strava',
}: SciotteLoginModalProps) {
  const { t } = useTranslation();
  const online = useOnlineStatus();
  // Credentials and a 2FA code, in an overlay that had no dialog semantics at
  // all: nothing announced it as a dialog, Escape did nothing, and Tab left
  // the password field for the page underneath.
  const titleId = useId();
  const { containerRef } = useDialog({ open: isOpen, onClose });
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
  // When the user picks t('app.useOwnStravaApp'), we open the BYO setup
  // modal on top of this one. On save it kicks off the official OAuth flow
  // and closes this whole stack via `onConnected`. Only meaningful when
  // `target === 'strava'` — Garmin and others don't expose an OAuth backend.
  const [showOAuthSetup, setShowOAuthSetup] = useState(false);
  // Server-configured login budget (DRAVR_SCIOTTE_LOGIN_TIMEOUT), fetched on
  // open. Null until loaded / on fetch failure — the progress copy degrades
  // to "a few minutes" rather than showing a hardcoded number.
  const [loginTimeoutSecs, setLoginTimeoutSecs] = useState<number | null>(null);

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
      oauthApi
        .sciotteConfig()
        .then((cfg) => setLoginTimeoutSecs(cfg.login_timeout_secs))
        .catch(() => setLoginTimeoutSecs(null));
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
      setStatus(`Connecting to ${target === 'garmin' ? t('shell.sciotteProviderGarmin') : t('shell.sciotteTargetStrava')}...`);

      try {
        const data = await oauthApi.sciotteLogin({ email, password, method, target });

        if (data.status === 'connected') {
          setPhase('success');
          setStatus('Connected!');
          onConnected();
          setTimeout(onClose, 1500);
        } else if (data.status === 'two_factor_choice') {
          setTwoFactorOptions(data.options || []);
          setPhase('two-factor');
          setStatus(t('app.chooseVerificationMethodShort'));
        } else if (data.status === 'otp_required') {
          setPhase('otp');
          setStatus(t('app.enterVerificationCode'));
          setOtpCode('');
        } else if (data.status === 'number_match') {
          // Defensive: only render the number-box UI when the server returned
          // an actual 2-3 digit number. Some upstream paths (Google /challenge/dp)
          // historically passed a placeholder string here, which made the modal
          // try to render it as a giant number and overflowed the box.
          const raw = data.number ?? null;
          const isRealNumber = !!raw && /^\d{2,3}$/.test(raw);
          setMatchNumber(isRealNumber ? raw : null);
          setPhase(isRealNumber ? 'number-match' : 'waiting-approval');
          setStatus(isRealNumber
            ? t('shell.sciotteTapMatchingNumber')
            : t('shell.sciotteApproveOnPhone'));
        } else {
          setError(data.error || t('auth.loginFailed'));
          setPhase('error');
        }
      } catch (err) {
        setError(describeApiError(err, { online, t, fallbackKey: 'auth.loginFailed' }));
        setPhase('error');
      } finally {
        setIsLoading(false);
      }
    },
    [email, password, method, target, onClose, onConnected, online, t]
  );

  // 2FA option selection
  const handleSelectTwoFactor = useCallback(
    async (optionId: string) => {
      setIsLoading(true);
      // Don't change phase for poll (number-match auto-poll) or app (waiting-approval)
      if (optionId === 'app') setPhase('waiting-approval');
      else if (optionId !== 'poll') setPhase('logging-in');
      setStatus(optionId === 'app' ? t('shell.sciotteCheckPhoneTapYes') : t('shell.sciotteLoadingStatus'));

      try {
        const data = await oauthApi.sciotteSelect2FA(optionId);

        if (data.status === 'connected') {
          setPhase('success');
          setStatus('Connected!');
          onConnected();
          setTimeout(onClose, 1500);
        } else if (data.status === 'otp_required') {
          setPhase('otp');
          setStatus(t('app.enterVerificationCode'));
          setOtpCode('');
        } else if (data.status === 'number_match') {
          // Defensive: only render the number-box UI when the server returned
          // an actual 2-3 digit number. Some upstream paths (Google /challenge/dp)
          // historically passed a placeholder string here, which made the modal
          // try to render it as a giant number and overflowed the box.
          const raw = data.number ?? null;
          const isRealNumber = !!raw && /^\d{2,3}$/.test(raw);
          setMatchNumber(isRealNumber ? raw : null);
          setPhase(isRealNumber ? 'number-match' : 'waiting-approval');
          setStatus(isRealNumber
            ? t('shell.sciotteTapMatchingNumber')
            : t('shell.sciotteApproveOnPhone'));
        } else {
          setError(data.error || t('shell.sciotteVerificationFailed'));
          setPhase('error');
        }
      } catch (err) {
        setError(describeApiError(err, { online, t, fallbackKey: 'shell.sciotteVerificationFailed' }));
        setPhase('error');
      } finally {
        setIsLoading(false);
      }
    },
    [onClose, onConnected, online, t]
  );

  // Auto-poll once when number-match phase is reached — the phone notification
  // arrives before the UI shows the number, so poll immediately
  const providerLabel = target === 'garmin' ? t('shell.sciotteProviderGarmin') : t('shell.sciotteTargetStrava');

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

  // Elapsed seconds + cycling status text while the sciotte browser runs.
  // The single-shot POST gives no real-time progress; this keeps the modal
  // visibly alive so it doesn't look frozen during the ~30s Chrome flow.
  const [elapsedSecs, setElapsedSecs] = useState(0);
  useEffect(() => {
    if (phase !== 'logging-in') {
      setElapsedSecs(0);
      return;
    }
    const startedAt = Date.now();
    const id = setInterval(() => {
      setElapsedSecs(Math.floor((Date.now() - startedAt) / 1000));
    }, 250);
    return () => clearInterval(id);
  }, [phase]);

  const progressLabel = (() => {
    if (phase !== 'logging-in') return status;
    if (elapsedSecs < 4) return 'Launching headless browser…';
    if (elapsedSecs < 10) return `Navigating to ${providerLabel}…`;
    if (elapsedSecs < 18) return 'Submitting credentials…';
    if (elapsedSecs < 28) return `Waiting for ${providerLabel} to respond…`;
    return 'Still working… provider login can be slow.';
  })();

  // OTP submission
  const handleOtpSubmit = useCallback(
    async (e: React.FormEvent) => {
      e.preventDefault();
      if (!otpCode) return;

      setIsLoading(true);
      setPhase('logging-in');
      setStatus(t('app.verifyingCode'));

      try {
        const data = await oauthApi.sciotteSubmitOTP(otpCode);

        if (data.status === 'connected') {
          setPhase('success');
          setStatus('Connected!');
          onConnected();
          setTimeout(onClose, 1500);
        } else if (data.status === 'otp_required') {
          setPhase('otp');
          setOtpCode('');
        } else {
          setError(data.error || t('shell.sciotteVerificationFailed'));
          setPhase('error');
        }
      } catch (err) {
        setError(describeApiError(err, { online, t, fallbackKey: 'shell.sciotteVerificationFailed' }));
        setPhase('error');
      } finally {
        setIsLoading(false);
      }
    },
    [otpCode, onClose, onConnected, online, t]
  );


  if (!isOpen) return null;

  const labels = target === 'garmin' && method === 'email'
    ? { titleKey: 'shell.sciotteGarminAccount', emailPlaceholderKey: 'shell.garminEmail' }
    : METHOD_LABELS[method];

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
      role="dialog"
      aria-modal="true"
      aria-labelledby={titleId}
    >
      <div
        ref={containerRef}
        tabIndex={-1}
        className="bg-surface-container-highest rounded-2xl border ghost-border shadow-2xl max-w-md w-full mx-4 overflow-hidden"
      >
        {/* Header */}
        <div className="flex items-center justify-between px-6 py-4 border-b ghost-border">
          <div className="flex items-center gap-3">
            {/* Strava orange, not a Boreal token. This is a third-party brand
                mark next to Strava's own logo path — recolouring it to
                `warning` would misrepresent the provider being connected. */}
            <div className="w-8 h-8 rounded-lg bg-gradient-to-br from-warning to-warning flex items-center justify-center flex-shrink-0">
              <svg className="w-4 h-4 text-on-surface" viewBox="0 0 24 24" fill="currentColor">
                <path d="M15.387 17.944l-2.089-4.116h-3.065L15.387 24l5.15-10.172h-3.066m-7.008-5.599l2.836 5.598h4.172L10.463 0l-7 13.828h4.169" />
              </svg>
            </div>
            <div>
              <h2 id={titleId} className="text-lg font-semibold text-on-surface">{t('frag.connectTo')} {providerLabel}</h2>
              {progressLabel && <p className="text-xs text-on-surface/50">{progressLabel}</p>}
            </div>
          </div>
          <button
            onClick={onClose}
            className="p-2 rounded-lg hover:bg-surface-container text-on-surface/60 hover:text-on-surface transition-colors"
            title={t('common.close')}
            aria-label={t('shell.sciotteCloseModalAria')}
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
                className="w-full flex items-center justify-center gap-3 px-4 py-3 bg-surface-container-high border ghost-border rounded-lg hover:bg-surface-container-highest hover:border-white/30 transition-all text-on-surface font-medium"
              >
                <svg className="w-5 h-5 flex-shrink-0" viewBox="0 0 24 24">
                  <path d="M22.56 12.25c0-.78-.07-1.53-.2-2.25H12v4.26h5.92a5.06 5.06 0 01-2.2 3.32v2.77h3.57c2.08-1.92 3.28-4.74 3.28-8.1z" fill="#4285F4" />
                  <path d="M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84C3.99 20.53 7.7 23 12 23z" fill="#34A853" />
                  <path d="M5.84 14.09c-.22-.66-.35-1.36-.35-2.09s.13-1.43.35-2.09V7.07H2.18C1.43 8.55 1 10.22 1 12s.43 3.45 1.18 4.93l2.85-2.22.81-.62z" fill="#FBBC05" />
                  <path d="M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15C17.45 2.09 14.97 1 12 1 7.7 1 3.99 3.47 2.18 7.07l3.66 2.84c.87-2.6 3.3-4.53 6.16-4.53z" fill="#EA4335" />
                </svg>
                {t('shell.sciotteContinueGoogle')}
              </button>

              <button
                onClick={() => selectMethod('apple')}
                className="w-full flex items-center justify-center gap-3 px-4 py-3 bg-surface-container-high border ghost-border rounded-lg hover:bg-surface-container-highest hover:border-white/30 transition-all text-on-surface font-medium"
              >
                <svg className="w-5 h-5 flex-shrink-0" viewBox="0 0 24 24" fill="currentColor">
                  <path d="M17.05 20.28c-.98.95-2.05.88-3.08.4-1.09-.5-2.08-.48-3.24 0-1.44.62-2.2.44-3.06-.4C2.79 15.25 3.51 7.59 9.05 7.31c1.35.07 2.29.74 3.08.8 1.18-.24 2.31-.93 3.57-.84 1.51.12 2.65.72 3.4 1.8-3.12 1.87-2.38 5.98.48 7.13-.57 1.5-1.31 2.99-2.54 4.09zM12.03 7.25c-.15-2.23 1.66-4.07 3.74-4.25.29 2.58-2.34 4.5-3.74 4.25z" />
                </svg>
                {t('shell.sciotteContinueApple')}
              </button>

              <div className="relative my-4">
                <div className="absolute inset-0 flex items-center"><div className="w-full border-t ghost-border" /></div>
                <div className="relative flex justify-center text-xs">
                  <span className="px-3 bg-surface-container-highest text-on-surface/50">Or</span>
                </div>
              </div>

              <button
                onClick={() => { setMethod('email'); setPhase('credentials'); }}
                className="w-full flex items-center justify-center gap-3 px-4 py-3 bg-surface-container-low border ghost-border rounded-lg hover:bg-surface-container hover:ghost-border transition-all text-on-surface/80 font-medium"
              >
                <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M3 8l7.89 5.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z" />
                </svg>
                {t('shell.sciotteLoginStravaEmail')}
              </button>

              {target === 'strava' && (
                <button
                  onClick={() => setShowOAuthSetup(true)}
                  className="w-full flex items-center justify-center gap-3 px-4 py-3 bg-surface-container-low border ghost-border rounded-lg hover:bg-surface-container hover:ghost-border transition-all text-on-surface/80 font-medium"
                >
                  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z" />
                  </svg>
                  {t('shell.sciotteUseOwnStravaApp')}
                </button>
              )}

              <p className="text-xs text-on-surface/30 text-center mt-3">
                {t('shell.sciotteEncrypted')}
              </p>
            </div>
          )}

          {/* Phase: Email credentials */}
          {phase === 'credentials' && (
            <div>
              <button onClick={() => target === 'garmin' ? onClose() : setPhase('choose')} className="flex items-center gap-1 text-sm text-on-surface/50 hover:text-on-surface/80 transition-colors mb-4">
                <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 19l-7-7 7-7" /></svg>
                {t('shell.sciotteBack')}
              </button>
              <h3 className="text-on-surface font-medium mb-4">{t(labels.titleKey)}</h3>
              <form onSubmit={handleEmailLogin} className="space-y-4">
                <div>
                  <label htmlFor="sciotte-email" className="block text-sm text-on-surface/60 mb-1.5">{t('common.email')}</label>
                  <input
                    id="sciotte-email"
                    type="email"
                    placeholder={t(labels.emailPlaceholderKey)}
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
                  <label htmlFor="sciotte-password" className="block text-sm text-on-surface/60 mb-1.5">{t('common.password')}</label>
                  <div className="relative">
                    <input
                      id="sciotte-password"
                      type={showPassword ? 'text' : 'password'}
                      placeholder={t('common.password')}
                      value={password}
                      onChange={(e) => setPassword(e.target.value)}
                      className="input-glass w-full pr-10"
                      required
                      autoComplete="current-password"
                      name="password"
                    />
                    <RevealButton
  revealed={showPassword}
  onToggle={() => setShowPassword(!showPassword)}
  label={showPassword ? t('auth.hidePassword') : t('auth.showPassword')}
/>
                  </div>
                </div>
                <button type="submit" disabled={isLoading || !email || !password} className="w-full py-3 bg-gradient-to-r from-nutrition to-warning rounded-lg text-on-surface font-medium hover:shadow-lg hover:shadow-nutrition/40 hover:-translate-y-0.5 transition-all disabled:opacity-50 disabled:cursor-not-allowed disabled:transform-none">
                  {isLoading ? t('shell.sciotteLoggingIn') : t('shell.sciotteLogIn')}
                </button>
              </form>
            </div>
          )}

          {/* Phase: Logging in */}
          {phase === 'logging-in' && (
            <div className="flex items-center justify-center py-12">
              <div className="text-center">
                <div className="pierre-spinner w-16 h-16 mx-auto mb-4 border-[3px] ghost-border border-t-on-surface" />
                <p className="text-on-surface/80 text-sm font-medium">{progressLabel}</p>
                <p className="text-on-surface/40 text-xs mt-2">
                  {(() => {
                    const budget = loginTimeoutSecs === null ? 'a few minutes' : formatTimeout(loginTimeoutSecs);
                    return elapsedSecs > 0
                      ? `${elapsedSecs}s elapsed — this may take up to ${budget}`
                      : t('frag.mayTakeUpTo', { budget });
                  })()}
                </p>
              </div>
            </div>
          )}

          {/* Phase: Two-factor choice */}
          {phase === 'two-factor' && (
            <div>
              <div className="text-center mb-4">
                <div className="w-12 h-12 mx-auto mb-3 rounded-full bg-warning/10 flex items-center justify-center">
                  <svg className="w-6 h-6 text-warning" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z" />
                  </svg>
                </div>
                <p className="text-on-surface font-medium">2-Step Verification</p>
                <p className="text-on-surface/50 text-sm mt-1">{t('shell.sciotteChooseVerification')}</p>
              </div>
              <div className="space-y-3">
                {twoFactorOptions.map((option) => (
                  <button
                    key={option.id}
                    onClick={() => handleSelectTwoFactor(option.id)}
                    disabled={isLoading}
                    className="w-full flex items-center gap-3 px-4 py-3 bg-surface-container-high border ghost-border rounded-lg hover:bg-surface-container-highest hover:border-white/30 transition-all text-on-surface text-left disabled:opacity-50"
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
                <div className="pierre-spinner w-12 h-12 mx-auto mb-4 border-2 ghost-border border-t-amber-500" />
                <p className="text-on-surface font-medium">{t('shell.sciotteCheckPhone')}</p>
                <p className="text-on-surface/50 text-sm mt-1">{t('shell.sciotteTapYes')}</p>
              </div>
            </div>
          )}

          {/* Phase: Number match challenge — auto-polls for approval */}
          {phase === 'number-match' && matchNumber && (
            <div className="flex items-center justify-center py-12">
              <div className="text-center">
                <div className="w-24 h-24 mx-auto mb-4 rounded-2xl bg-gradient-to-br from-info/20 to-info/10 border-2 border-info/40 flex items-center justify-center">
                  <span className="text-5xl font-bold text-info">{matchNumber}</span>
                </div>
                <p className="text-on-surface font-medium mb-1">{t('shell.sciotteTapNumber')}</p>
                <p className="text-on-surface/50 text-sm mb-4">{t('shell.sciotteGoogleNotificationHint')}</p>
                <div className="pierre-spinner w-8 h-8 mx-auto border-2 ghost-border border-t-blue-500" />
                <p className="text-on-surface/30 text-xs mt-3">{t('shell.sciotteWaitingForTap')}</p>
              </div>
            </div>
          )}

          {/* Phase: OTP */}
          {phase === 'otp' && (
            <div>
              <div className="text-center mb-4">
                <div className="w-12 h-12 mx-auto mb-3 rounded-full bg-warning/10 flex items-center justify-center">
                  <svg className="w-6 h-6 text-warning" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z" />
                  </svg>
                </div>
                <p className="text-on-surface font-medium">{t('shell.sciotteVerificationRequired')}</p>
                <p className="text-on-surface/50 text-sm mt-1">{t('shell.sciotteEnterCode')}</p>
              </div>
              <form onSubmit={handleOtpSubmit} className="space-y-4">
                <input type="text" placeholder={t('shell.sciotteVerificationCode')} value={otpCode} onChange={(e) => setOtpCode(e.target.value)} className="input-glass w-full text-center text-lg tracking-widest" required autoFocus autoComplete="one-time-code" inputMode="numeric" />
                <button type="submit" disabled={isLoading || !otpCode} className="w-full py-3 bg-gradient-to-r from-nutrition to-warning rounded-lg text-on-surface font-medium hover:shadow-lg hover:shadow-nutrition/40 hover:-translate-y-0.5 transition-all disabled:opacity-50 disabled:cursor-not-allowed disabled:transform-none">
                  {t('shell.sciotteVerify')}
                </button>
              </form>
            </div>
          )}

          {/* Phase: Success */}
          {phase === 'success' && (
            <div className="flex items-center justify-center py-12">
              <div className="text-center">
                <div className="w-16 h-16 mx-auto mb-4 rounded-full bg-activity/10 flex items-center justify-center">
                  <svg className="w-8 h-8 text-activity" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" /></svg>
                </div>
                <p className="text-activity text-lg font-medium">{t('shell.sciotteConnected')}</p>
                <p className="text-on-surface/50 text-sm mt-1">{t('frag.your')} {providerLabel} data is now available</p>
              </div>
            </div>
          )}

          {/* Phase: Error */}
          {phase === 'error' && (
            <div className="flex items-center justify-center py-12">
              <div className="text-center">
                <div className="w-16 h-16 mx-auto mb-4 rounded-full bg-error/10 flex items-center justify-center">
                  <svg className="w-8 h-8 text-error" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-2.5L13.732 4c-.77-.833-1.964-.833-2.732 0L4.082 16.5c-.77.833.192 2.5 1.732 2.5z" /></svg>
                </div>
                <p className="text-error text-lg font-medium mb-2">{t('shell.sciotteLoginFailed')}</p>
                <p className="text-on-surface/50 text-sm max-w-sm mb-4">{error}</p>
                <button onClick={() => { setPhase(target === 'garmin' ? 'credentials' : 'choose'); setError(null); }} className="px-4 py-2 bg-surface-container-high border ghost-border hover:bg-surface-container-highest hover:border-white/30 rounded-lg text-on-surface text-sm transition-all">
                  {t('chat.tryAgain')}
                </button>
              </div>
            </div>
          )}
        </div>
      </div>

      <OAuthAppSetupModal
        isOpen={showOAuthSetup}
        onClose={() => setShowOAuthSetup(false)}
        onSaved={async () => {
          setShowOAuthSetup(false);
          // Kick off the official Strava OAuth flow now that BYO credentials
          // are stored. Same window.open dance ProviderConnectionCards uses
          // for the OAuth providers — must fire from a click-adjacent stack
          // so popup blockers stay quiet on Safari.
          //
          // NOTE: we deliberately do NOT call `onConnected()` here. The user
          // has only just been redirected into Strava's consent screen; they
          // haven't authorized yet. Connection is confirmed at the App level
          // when the OAuth callback comes back and `App.tsx` invalidates the
          // onboarding-status query (see `getOAuthCallbackParams` there).
          // Calling `onConnected()` prematurely flashed "Provider connected
          // — preparing your dashboard…" before the user had even seen the
          // consent screen.
          const popup = window.open('about:blank', '_blank');
          try {
            const authUrl = await oauthApi.getAuthorizeUrlForProvider('strava');
            if (popup && !popup.closed) {
              popup.location.href = authUrl;
            } else {
              window.location.href = authUrl;
            }
            // Close the Sciotte modal so the user lands back on the
            // onboarding cards while the popup handles Strava consent. The
            // popup's redirect (or its absence) is the source of truth.
            onClose();
            // Tell the parent an OAuth popup is in flight so it can render an
            // "awaiting consent" state with a cancel + timeout escape hatch.
            onOAuthLaunched?.('strava');
          } catch (err) {
            if (popup && !popup.closed) popup.close();
            setError(describeApiError(err, { online, t, fallbackKey: 'shell.sciotteStravaOauthFailed' }));
            setPhase('error');
          }
        }}
        provider="strava"
        displayName="Strava"
        devPortalUrl="https://www.strava.com/settings/api"
      />
    </div>
  );
}
