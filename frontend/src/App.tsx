// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState, useEffect, useRef } from 'react';
import { QueryClient, QueryClientProvider, useQuery, useQueryClient } from '@tanstack/react-query';
import Login from './components/Login';
import Register from './components/Register';
import ForgotPassword from './components/ForgotPassword';
import ResetPassword from './components/ResetPassword';
import PendingApproval from './components/PendingApproval';
import Dashboard from './components/Dashboard';
import OnboardingConnectProvider from './components/OnboardingConnectProvider';
import OnboardingCoachProposal from './components/OnboardingCoachProposal';
import OnboardingProfileType from './components/OnboardingProfileType';
import ImpersonationBanner from './components/ImpersonationBanner';
import OAuthCallback from './components/OAuthCallback';
import ErrorBoundary from './components/ErrorBoundary';
import { AuthProvider } from './contexts/AuthContext';
import { ToastProvider } from './components/ui';
import { ThemeProvider } from './hooks/useTheme';
import { useAuth } from './hooks/useAuth';
import { QUERY_KEYS } from './constants/queryKeys';
import { userApi } from './services/api';
import './App.css';

const queryClient = new QueryClient();

/**
 * Check if the current URL has OAuth callback parameters
 */
function getOAuthCallbackParams(): { provider: string; success: boolean; error?: string } | null {
  const urlParams = new URLSearchParams(window.location.search);
  const provider = urlParams.get('provider');
  const success = urlParams.get('success');

  if (provider && success !== null) {
    return {
      provider,
      success: success === 'true',
      error: urlParams.get('error') || undefined,
    };
  }
  return null;
}

/**
 * Check if the current URL is an invite link: /groups/join/:code
 */
function getGroupInviteCode(): string | null {
  const match = window.location.pathname.match(/^\/groups\/join\/([^/]+)$/);
  return match ? decodeURIComponent(match[1]) : null;
}

type AuthView = 'login' | 'register' | 'forgot-password' | 'reset-password';

function AppContent() {
  const { user, isAuthenticated, isLoading } = useAuth();
  const [authView, setAuthView] = useState<AuthView>('login');
  const [registrationMessage, setRegistrationMessage] = useState<string | null>(null);
  const [registeredEmail, setRegisteredEmail] = useState<string>('');
  const [resetEmail, setResetEmail] = useState<string>('');
  const [oauthCallback, setOauthCallback] = useState<{ provider: string; success: boolean; error?: string } | null>(null);
  const [pendingInviteCode, setPendingInviteCode] = useState<string | null>(null);
  const localQueryClient = useQueryClient();

  // Onboarding gate: a fresh account with zero `provider_connections` rows
  // cannot reach chat/coach without the server returning a 403, so route
  // those users to the connect-provider screen instead of the dashboard.
  // The query only fires once the user is active — pending / suspended
  // states get their own screens upstream of this check.
  //
  // Admins are exempt: their primary intent is administering, not chatting,
  // so a missing provider on the admin account must not block them from
  // the dashboard / settings / admin panels. The backend 403 still applies
  // if an admin tries to chat without a provider — same hallucination guard,
  // just no forced redirect off the entire UI.
  // (See `feedback_spa_gate_on_role_not_is_admin`: gate on `user.role`.)
  const isAdminRole = user?.role === 'admin' || user?.role === 'super_admin';
  const onboardingActive =
    isAuthenticated && user?.user_status === 'active' && !isAdminRole;
  const { data: onboardingStatus } = useQuery({
    queryKey: QUERY_KEYS.user.onboardingStatus(),
    queryFn: () => userApi.getOnboardingStatus(),
    enabled: onboardingActive,
    // Keep this fresh during the OAuth round-trip so the redirect flips as
    // soon as the provider connection lands.
    staleTime: 5_000,
  });

  // Session-only "continue without a provider": lets the user into the app
  // without connecting. Deliberately not persisted (no skip flag) — the connect
  // prompts in chat and on the coach screens carry the nudge from here on.
  const [skippedOnboarding, setSkippedOnboarding] = useState(false);

  // Coach-proposal step: shown once, right after the provider connection lands
  // and before the dashboard, to drive the
  // `connect → analyzing → profile → coaches` onboarding flow.
  //
  // It fires ONLY when the user connects their first provider *in this session*
  // — detected as a `needs_provider_connection` true→false transition — so it
  // never intercepts an already-onboarded user simply logging in (or an E2E
  // run that authenticates a provider-connected fixture user). Completion is
  // also persisted per-user in localStorage as a belt-and-suspenders guard.
  const coachProposalKey = user?.user_id ? `dravr.coach_proposal_done.${user.user_id}` : null;
  const [coachProposalDone, setCoachProposalDone] = useState(false);
  useEffect(() => {
    setCoachProposalDone(coachProposalKey ? localStorage.getItem(coachProposalKey) === '1' : true);
  }, [coachProposalKey]);

  // Profile-type step: a fresh, not-yet-connected user picks athlete vs coach
  // before the connect-provider screen. Gated on `needs_provider_connection`
  // (below) so an already-onboarded user never sees it, plus a per-user
  // localStorage flag so it shows once within the first-run connect flow.
  const profileTypeKey = user?.user_id ? `dravr.profile_type_chosen.${user.user_id}` : null;
  const [profileTypeChosen, setProfileTypeChosen] = useState(false);
  useEffect(() => {
    setProfileTypeChosen(profileTypeKey ? localStorage.getItem(profileTypeKey) === '1' : true);
  }, [profileTypeKey]);

  const [justOnboarded, setJustOnboarded] = useState(false);
  const prevNeedsProvider = useRef<boolean | undefined>(undefined);
  useEffect(() => {
    const now = onboardingStatus?.needs_provider_connection;
    if (prevNeedsProvider.current === true && now === false) {
      setJustOnboarded(true);
    }
    prevNeedsProvider.current = now;
  }, [onboardingStatus?.needs_provider_connection]);

  // Boot/teardown PostHog analytics in lockstep with the user's
  // analytics_consent flag. Default is opt-out so this is a no-op
  // until the user enables it under Privacy & Data.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const mod = await import('./services/analytics');
      if (cancelled) return;
      if (user && user.analytics_consent === true) {
        mod.bootAnalytics(user.id, true);
      } else {
        mod.shutdownAnalytics();
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [user?.id, user?.analytics_consent]);

  // Check for OAuth callback params on mount
  useEffect(() => {
    const params = getOAuthCallbackParams();
    if (params) {
      setOauthCallback(params);
      // Invalidate OAuth status queries to refresh connection state.
      // Also invalidate onboarding-status so a first-run connect flips the
      // route guard from `<OnboardingConnectProvider>` to `<Dashboard>`
      // without a page reload.
      localQueryClient.invalidateQueries({ queryKey: QUERY_KEYS.oauth.status() });
      localQueryClient.invalidateQueries({ queryKey: QUERY_KEYS.oauth.connections() });
      localQueryClient.invalidateQueries({ queryKey: QUERY_KEYS.providers.status() });
      localQueryClient.invalidateQueries({ queryKey: QUERY_KEYS.user.onboardingStatus() });
    }
  }, [localQueryClient]);

  // Check for group invite link on mount: /groups/join/:code
  useEffect(() => {
    const code = getGroupInviteCode();
    if (code) {
      setPendingInviteCode(code);
      // Clean the URL so the invite path doesn't persist on refresh
      window.history.replaceState({}, document.title, '/');
    }
  }, []);

  // Show OAuth callback result page
  if (oauthCallback) {
    return (
      <OAuthCallback
        provider={oauthCallback.provider}
        success={oauthCallback.success}
        error={oauthCallback.error}
        onClose={() => {
          // Dispatch event for ChatTab to detect OAuth completion
          window.postMessage({
            type: 'oauth_completed',
            provider: oauthCallback.provider,
            success: oauthCallback.success,
          }, window.location.origin);
          // Clear URL params and close the callback view
          window.history.replaceState({}, document.title, window.location.pathname);
          setOauthCallback(null);
        }}
      />
    );
  }

  if (isLoading) {
    return (
      <div className="min-h-dvh bg-surface flex items-center justify-center">
        <div className="text-center">
          <div className="animate-spin rounded-full h-32 w-32 border-b-2 border-primary mx-auto"></div>
          <p className="mt-4 text-on-surface-variant">Loading...</p>
        </div>
      </div>
    );
  }

  // Not authenticated - show login, register, or forgot/reset password
  if (!isAuthenticated) {
    if (authView === 'register') {
      return (
        <Register
          onNavigateToLogin={() => {
            setAuthView('login');
            setRegistrationMessage(null);
          }}
          onRegistrationSuccess={(message, email) => {
            setRegistrationMessage(message);
            setRegisteredEmail(email);
            setAuthView('login');
          }}
        />
      );
    }

    if (authView === 'forgot-password') {
      return (
        <ForgotPassword
          onNavigateToLogin={() => setAuthView('login')}
          onCodeSent={(email) => {
            setResetEmail(email);
            setAuthView('reset-password');
          }}
        />
      );
    }

    if (authView === 'reset-password') {
      return (
        <ResetPassword
          email={resetEmail}
          onNavigateToLogin={() => setAuthView('login')}
          onResetSuccess={(message) => {
            setRegistrationMessage(message);
            setAuthView('login');
          }}
          onResendCode={() => setAuthView('forgot-password')}
        />
      );
    }

    return (
      <div className="min-h-dvh bg-surface">
        {registrationMessage && (
          <div className="fixed top-4 left-1/2 transform -translate-x-1/2 z-50 max-w-md w-full px-4">
            <div className="bg-pierre-activity/20 border border-pierre-activity text-on-surface px-4 py-3 rounded-lg shadow-lg backdrop-blur-sm">
              <div className="flex items-center justify-between">
                <p className="text-sm font-medium">{registrationMessage}</p>
                <button
                  onClick={() => setRegistrationMessage(null)}
                  className="ml-4 text-on-surface-variant hover:text-on-surface"
                >
                  <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                  </svg>
                </button>
              </div>
            </div>
          </div>
        )}
        <Login
          onNavigateToRegister={() => setAuthView('register')}
          onNavigateToForgotPassword={() => setAuthView('forgot-password')}
          prefilledEmail={registeredEmail || undefined}
        />
      </div>
    );
  }

  // Authenticated but pending approval
  if (user?.user_status === 'pending') {
    return <PendingApproval />;
  }

  // Authenticated but suspended
  if (user?.user_status === 'suspended') {
    return (
      <div className="min-h-dvh bg-surface flex items-center justify-center px-4">
        <div className="max-w-md w-full bg-surface-container-low/60 backdrop-blur-xl border ghost-border rounded-2xl overflow-hidden">
          <div className="h-1 w-full bg-gradient-to-r from-red-500 to-red-600" />
          <div className="px-8 py-10 text-center">
            <svg className="w-16 h-16 text-red-500 mx-auto mb-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M18.364 18.364A9 9 0 005.636 5.636m12.728 12.728A9 9 0 015.636 5.636m12.728 12.728L5.636 5.636" />
            </svg>
            <h1 className="text-xl font-bold text-on-surface mb-2">Account Suspended</h1>
            <p className="text-sm text-on-surface-variant mb-6">
              Your account has been suspended. Please contact an administrator for assistance.
            </p>
          </div>
        </div>
      </div>
    );
  }

  // Render the dashboard immediately while onboarding-status is in flight.
  // Only switch to the connect-provider screen when the server explicitly
  // confirms `needs_provider_connection: true` — flashing a full-screen
  // spinner first races E2E tests against UI elements that don't render
  // until the query resolves, and a one-frame flash of dashboard chrome
  // before a redirect is acceptable on the first-login edge case.
  // Profile-type onboarding step: first run, before the connect-provider gate.
  // Only a fresh account (needs a provider) that hasn't chosen yet sees this —
  // already-onboarded users (provider connected) skip straight past it.
  if (
    onboardingActive &&
    onboardingStatus?.needs_provider_connection === true &&
    !skippedOnboarding &&
    profileTypeKey &&
    !profileTypeChosen
  ) {
    return (
      <div className="min-h-dvh bg-surface">
        <ImpersonationBanner />
        <OnboardingProfileType
          userDisplayName={user?.display_name}
          onComplete={() => {
            localStorage.setItem(profileTypeKey, '1');
            setProfileTypeChosen(true);
          }}
        />
      </div>
    );
  }

  if (onboardingStatus?.needs_provider_connection === true && !skippedOnboarding) {
    return (
      <div className="min-h-dvh bg-surface">
        <ImpersonationBanner />
        <OnboardingConnectProvider
          userDisplayName={user?.display_name}
          onContinueWithoutProvider={() => setSkippedOnboarding(true)}
        />
      </div>
    );
  }

  // Coach-proposal onboarding step: a provider is connected (needs === false)
  // but the user hasn't seen their inferred-profile coach suggestions yet.
  if (
    onboardingActive &&
    justOnboarded &&
    onboardingStatus?.needs_provider_connection === false &&
    coachProposalKey &&
    !coachProposalDone
  ) {
    return (
      <div className="min-h-dvh bg-surface">
        <ImpersonationBanner />
        <OnboardingCoachProposal
          userDisplayName={user?.display_name}
          onComplete={() => {
            localStorage.setItem(coachProposalKey, '1');
            setCoachProposalDone(true);
            setJustOnboarded(false);
          }}
        />
      </div>
    );
  }

  // Authenticated and active - show dashboard
  return (
    <div className="min-h-dvh bg-surface">
      <ImpersonationBanner />
      <Dashboard pendingInviteCode={pendingInviteCode} onInviteCodeConsumed={() => setPendingInviteCode(null)} />
    </div>
  );
}

function App() {
  return (
    <ErrorBoundary
      onError={(error, errorInfo) => {
        // Log errors to console in development
        // In production, this could send to an error tracking service
        console.error('Application error:', error);
        console.error('Component stack:', errorInfo.componentStack);
      }}
    >
      <QueryClientProvider client={queryClient}>
        <ThemeProvider>
          <AuthProvider>
            <ToastProvider>
              <AppContent />
            </ToastProvider>
          </AuthProvider>
        </ThemeProvider>
      </QueryClientProvider>
    </ErrorBoundary>
  );
}

export default App;
