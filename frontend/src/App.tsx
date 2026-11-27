// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Root of the web app: reads the links a page load arrives on (OAuth return, invitation, confirmation, signup)
// ABOUTME: Gates on authentication and account status, then renders onboarding or the dashboard inside the app providers

import { lazy, Suspense, useState, useEffect } from 'react';
import { QueryClientProvider } from '@tanstack/react-query';
import Login from './components/Login';
import Register from './components/Register';
import ForgotPassword from './components/ForgotPassword';
import ResetPassword from './components/ResetPassword';
import VerifyEmail from './components/VerifyEmail';
import AcceptInvitation from './components/AcceptInvitation';
import { ACCEPT_INVITATION_PATH, TAKEOVER_PATH, VERIFY_EMAIL_PATH, parseInvitationKind, parseLinkToken } from './utils/tokenLink';
import { isRegisterLandingPath, parseSignupCode } from './utils/signupLink';
import PendingApproval from './components/PendingApproval';
import Dashboard from './components/Dashboard';
import OnboardingFlow from './components/OnboardingFlow';
import ImpersonationBanner from './components/ImpersonationBanner';
import OfflineBanner from './components/OfflineBanner';
import InstallAppBanner from './components/InstallAppBanner';
import ServiceWorkerUpdatePrompt from './components/ServiceWorkerUpdatePrompt';
import OAuthCallback from './components/OAuthCallback';
import ImportWindow from './components/import/ImportWindow';
import { IMPORT_PATH } from './components/import/openImportWindow';
import ErrorBoundary from './components/ErrorBoundary';
import UnitsProvider from './components/UnitsProvider';
import { AuthProvider } from './contexts/AuthContext';
import { ALERT_TONE, ToastProvider } from './components/ui';
import { ThemeProvider } from './hooks/useTheme';
import { useAuth } from './hooks/useAuth';
import { useOnboardingState } from './hooks/useOnboardingState';
import { useHeldGroupInvite } from './hooks/useHeldGroupInvite';
import { useIdleWatch } from './hooks/useIdleWatch';
import { useApiRefusalSurface } from './hooks/useApiRefusalSurface';
import { useLanguageSwitcher, useTranslation } from '@pierre/i18n';
import { hasAdminRole } from '@pierre/shared-types';
import { importTombstoneFromReturn } from '@pierre/ui-logic';
import { returnToConnectStart, settleOAuthReturn, type OAuthCallbackParams } from './utils/oauthReturn';
import { queryClient } from './services/queryClient';
import { dismissInviteCard, readCarriedGroupInvite, useInviteCardOpen } from './utils/groupInvite';
import './App.css';

/** The login takeover page (ADR-034), split out of the entry bundle. */
const SignInCheck = lazy(() => import('./components/SignInCheck'));
// Only an admin ever meets the mode picker, so it stays out of the entry bundle.
const ViewModePicker = lazy(() => import('./components/ViewModePicker'));
// Only a load that opened on an invite link shows the invite card (carnet#1649).
const InviteCard = lazy(() => import('./components/InviteCard'));

/**
 * Check if the current URL has OAuth callback parameters
 */
function getOAuthCallbackParams(): OAuthCallbackParams | null {
  const urlParams = new URLSearchParams(window.location.search);
  const provider = urlParams.get('provider');
  const success = urlParams.get('success');

  if (provider && success !== null) {
    return {
      provider,
      success: success === 'true',
      error: urlParams.get('error') || undefined,
      tombstone: importTombstoneFromReturn((name) => urlParams.get(name)),
    };
  }
  return null;
}

/**
 * Whether the current URL is a mailed address-confirmation link: /verify-email#token=…
 */
function isVerifyEmailPath(): boolean {
  return window.location.pathname === VERIFY_EMAIL_PATH;
}

/**
 * Whether the current URL is a mailed invitation: /accept-invite#token=…
 */
function isAcceptInvitationPath(): boolean {
  return window.location.pathname === ACCEPT_INVITATION_PATH;
}

/**
 * Whether the current URL is a login takeover: /sign-in-check#token=…
 */
function isTakeoverPath(): boolean {
  return window.location.pathname === TAKEOVER_PATH;
}

/**
 * Whether the current URL is the create-account link: /register, or /signup.
 * dravr.ai's "Create your account" button points here, so it opens the form it
 * names rather than the sign-in form.
 */
function isRegisterPath(): boolean {
  return isRegisterLandingPath(window.location.pathname);
}

/**
 * The signup code a create-account link carries (`/signup#code=…`, or an
 * older `/signup?code=…`), so a code an admin or coach shares as a link fills
 * itself in (carnet#1128, carnet#1873).
 */
function getSignupCodeParam(): string | null {
  if (!isRegisterPath()) return null;
  return parseSignupCode(window.location.search, window.location.hash);
}

type AuthView = 'login' | 'register' | 'forgot-password' | 'reset-password';

function AppContent() {
  const { t } = useTranslation();
  const { user, isAuthenticated, isLoading, viewMode, impersonation } = useAuth();
  const [authView, setAuthView] = useState<AuthView>(() => (isRegisterPath() ? 'register' : 'login'));
  const [registrationMessage, setRegistrationMessage] = useState<string | null>(null);
  // Read once, before the effect below cleans the create-account link.
  const [signupCodeFromLink] = useState<string | null>(getSignupCodeParam);
  const [resetEmail, setResetEmail] = useState<string>('');
  // A connect returns to the app root, in the tab it started from. Settled
  // before the first render, so the dashboard opens on the route the connect
  // started from rather than on the root it came back to; only a refusal with
  // something to say keeps the callback page.
  const [oauthCallback, setOauthCallback] = useState<OAuthCallbackParams | null>(() =>
    settleOAuthReturn(getOAuthCallbackParams()),
  );
  // The confirmation token, read once from the link's fragment. `undefined`
  // when this page load is not a confirmation link.
  const [verifyEmailToken, setVerifyEmailToken] = useState<string | null | undefined>(() =>
    isVerifyEmailPath() ? parseLinkToken(window.location.hash) : undefined,
  );
  // The invitation token, read once from the link's fragment. `undefined`
  // when this page load is not an invitation link.
  const [invitationToken, setInvitationToken] = useState<string | null | undefined>(() =>
    isAcceptInvitationPath() ? parseLinkToken(window.location.hash) : undefined,
  );
  // What the invitation link says it is (a coach seat words the page for a
  // coach), read with the token before the fragment is cleared.
  const [invitationKind] = useState(() => parseInvitationKind(window.location.hash));
  // The takeover token, read once from the link's fragment. `undefined` when
  // this page load is not a takeover link.
  const [takeoverToken] = useState<string | null | undefined>(() =>
    isTakeoverPath() ? parseLinkToken(window.location.hash) : undefined,
  );

  // Adopt the viewer's language for the whole app, not just the screen that
  // owns the picker. The restore effect lives in the switcher hook, and the
  // switcher only renders inside Settings, so a viewer who chose a language
  // and reloaded onto Chat got the default one back everywhere else — a gap
  // that stayed invisible only because the chrome was hardcoded English.
  // Mounting the same hook here means one restore path, not a parallel one.
  const { currentLanguage } = useLanguageSwitcher({ serverLocale: user?.locale });

  // Keep `<html lang>` on the language actually rendered. index.html ships
  // `lang="en"` and nothing moved it, so French copy was announced to screen
  // readers as English and offered to translation tooling as English.
  useEffect(() => {
    document.documentElement.lang = currentLanguage;
  }, [currentLanguage]);

  // Stop polling — and drop any open turn stream — once the tab has gone
  // untouched for long enough that nobody is reading it.
  useIdleWatch();

  // Give an authorization refusal somewhere to land. Mounted here because this
  // is the app's one component inside both AuthProvider and ToastProvider, and
  // because the refusals it answers arrive on tabs that render nothing else.
  useApiRefusalSurface();

  // Onboarding flow state (server status + per-step flags + transitions). Kept
  // here — above the `oauthCallback` early return — so the status query stays
  // mounted across the OAuth-callback screen and the needs→false transition
  // that keeps the connect step up for a second service is observed. A group
  // invite link is a coach inviting an athlete: the role is recorded from it.
  // The invite itself is settled here rather than in the dashboard, which
  // mounts only after onboarding: the server holds it from the first session,
  // and the join runs as soon as the account is active (carnet#1648).
  const heldInvite = useHeldGroupInvite();
  // A page load that opened on a /groups/join/<code> link starts on the
  // invite card, signed in or not; the code it names is read once (carnet#1649).
  const inviteCardOpen = useInviteCardOpen();
  const [inviteCardCode] = useState<string | null>(readCarriedGroupInvite);
  // A session the card cannot serve skips it: an admin impersonating (that
  // session is the target's), a suspended account, or no code left to read.
  const cardUnservable =
    impersonation.isImpersonating || user?.user_status === 'suspended' || inviteCardCode === null;
  useEffect(() => {
    if (inviteCardOpen && !isLoading && cardUnservable) dismissInviteCard();
  }, [inviteCardOpen, isLoading, cardUnservable]);
  const onboarding = useOnboardingState({ invitedAsAthlete: heldInvite.invitedAsAthlete });

  // Boot/teardown PostHog analytics in lockstep with the user's
  // analytics_consent flag. Default is opt-out so this is a no-op
  // until the user enables it under Privacy & Data.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const mod = await import('./services/analytics');
      if (cancelled) return;
      if (user && user.analytics_consent === true) {
        await mod.bootAnalytics(user.id, true);
      } else {
        mod.shutdownAnalytics();
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [user?.id, user?.analytics_consent]);

  // Clean the create-account link once the form is chosen, so a reload after
  // switching to sign-in does not reopen it and a signed-in user lands on '/'.
  // The signup code it may carry is already in state, and leaves the address
  // bar with the rest of the link.
  useEffect(() => {
    if (isRegisterPath()) {
      window.history.replaceState({}, document.title, '/');
    }
  }, []);

  // A session ends on the sign-in form, whichever form was showing before it
  // began: a signed-in visitor who followed the create-account link never asked
  // for that one. Waits for the restore to settle, because the cached user counts
  // as signed in until then and a dead session must keep the form the link chose.
  useEffect(() => {
    if (!isLoading && isAuthenticated) {
      setAuthView('login');
    }
  }, [isLoading, isAuthenticated]);

  // The invitation and confirmation tokens are single-use credentials: once
  // read into state, they leave the address bar and the history entry, so a
  // reload or a shared screenshot of the URL cannot carry them anywhere.
  useEffect(() => {
    if (isAcceptInvitationPath() || isVerifyEmailPath()) {
      window.history.replaceState({}, document.title, '/');
    }
    // The takeover token leaves the address bar too, but the page stays: a
    // reload there shows that the sign-in ended (closing its socket ended
    // it), never the dashboard.
    if (isTakeoverPath()) {
      window.history.replaceState({}, document.title, TAKEOVER_PATH);
    }
  }, []);

  // Login takeover (ADR-034). Rendered before every auth branch: the hosted
  // sign-in pages and the phone app's WebView open it with no session, and
  // its token is all the takeover socket asks for.
  // Loaded on demand: only a takeover link opens it, so its viewer stays out
  // of every other page's first paint.
  if (takeoverToken !== undefined) {
    return (
      <Suspense fallback={null}>
        <SignInCheck token={takeoverToken} />
      </Suspense>
    );
  }

  // Invitation landing. Rendered before every auth branch for the same reason
  // as the email-confirmation landing below: the invitee arrives from a mail
  // client with no account yet, so nothing behind a sign-in can serve them.
  if (invitationToken !== undefined) {
    return (
      <AcceptInvitation
        token={invitationToken}
        kind={invitationKind}
        onSignIn={() => {
          setInvitationToken(undefined);
          setAuthView('login');
        }}
        onRegister={() => {
          setInvitationToken(undefined);
          setAuthView('register');
        }}
      />
    );
  }

  // Email-confirmation landing. Rendered before every auth branch: the user
  // arrives here from their mail client, usually signed out and often on a
  // different device from the one they registered on, so gating it behind the
  // login form would strand them on the step that was meant to unblock them.
  if (verifyEmailToken !== undefined) {
    return (
      <VerifyEmail
        token={verifyEmailToken}
        onContinue={() => {
          window.history.replaceState({}, document.title, '/');
          setVerifyEmailToken(undefined);
        }}
      />
    );
  }

  // Show OAuth callback result page
  if (oauthCallback) {
    return (
      <OAuthCallback
        provider={oauthCallback.provider}
        error={oauthCallback.error}
        tombstone={oauthCallback.tombstone}
        onClose={() => {
          returnToConnectStart();
          setOauthCallback(null);
        }}
      />
    );
  }

  if (isLoading) {
    return (
      <div className="min-h-dvh bg-surface flex items-center justify-center pad-safe-top">
        {/* role=status so the wait is announced; the spinner itself is decorative.
            A 128px ring dominated the viewport on a phone — 48px reads as a
            spinner rather than as a broken layout. */}
        <div className="text-center" role="status" aria-live="polite">
          <div
            aria-hidden="true"
            className="animate-spin rounded-full h-12 w-12 border-b-2 border-primary mx-auto"
          />
          <p className="mt-4 text-on-surface-variant">{t('common.loading')}</p>
        </div>
      </div>
    );
  }

  // The invite card: before any form, before any hold or join. Its buttons are
  // the person's answer — a form opens, or the join runs — and only then does
  // the page carry on as every other load does.
  if (inviteCardOpen && !cardUnservable && inviteCardCode !== null) {
    return (
      <Suspense
        fallback={
          <div className="min-h-dvh flex items-center justify-center bg-surface" role="status" aria-live="polite">
            <div aria-hidden="true" className="animate-spin rounded-full h-12 w-12 border-b-2 border-primary" />
          </div>
        }
      >
        <InviteCard
          code={inviteCardCode}
          signedIn={isAuthenticated}
          onCreateAccount={() => {
            setAuthView('register');
            dismissInviteCard();
          }}
          onSignIn={() => {
            setAuthView('login');
            dismissInviteCard();
          }}
          onJoin={heldInvite.joinFromCard}
          onDone={dismissInviteCard}
        />
      </Suspense>
    );
  }

  // Not authenticated - show login, register, or forgot/reset password
  if (!isAuthenticated) {
    if (authView === 'register') {
      return (
        <Register
          initialSignupCode={signupCodeFromLink ?? undefined}
          onNavigateToLogin={() => {
            setAuthView('login');
            setRegistrationMessage(null);
          }}
          onRegistrationSuccess={(message) => {
            setRegistrationMessage(message);
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
        <OfflineBanner />
        {registrationMessage && (
          <div
            className="fixed top-4 left-1/2 transform -translate-x-1/2 z-50 max-w-md w-full px-4 pad-safe-top"
          >
            <div className={`${ALERT_TONE.success.tone} border px-4 py-3 rounded-lg backdrop-blur-sm`}>
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
      <div className="min-h-dvh bg-surface flex items-center justify-center px-4 pad-safe-top">
        <div className="max-w-md w-full bg-surface-container-low/60 backdrop-blur-xl border ghost-border rounded-2xl overflow-hidden">
          <div className="h-1 w-full bg-error" />
          <div className="px-8 py-10 text-center">
            <svg className="w-16 h-16 text-error mx-auto mb-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M18.364 18.364A9 9 0 005.636 5.636m12.728 12.728A9 9 0 015.636 5.636m12.728 12.728L5.636 5.636" />
            </svg>
            <h1 className="text-xl font-bold text-on-surface mb-2">
              {t('shell.accountSuspendedTitle')}
            </h1>
            <p className="text-sm text-on-surface-variant mb-6">
              {t('shell.accountSuspendedBody')}
            </p>
          </div>
        </div>
      </div>
    );
  }

  // An admin chooses, at every sign-in, between the admin console and the
  // athlete app on their own account (carnet#1631). Asked before onboarding,
  // which the athlete mode goes through like any athlete. Never while
  // impersonating: that session is the target's.
  if (hasAdminRole(user) && viewMode === null && !impersonation.isImpersonating) {
    return (
      <div className="min-h-dvh bg-surface">
        <OfflineBanner />
        <Suspense fallback={null}>
          <ViewModePicker />
        </Suspense>
      </div>
    );
  }

  // The import window (carnet#1218): opened from the provider list in a
  // window of its own, or in place in the installed app. It comes before
  // onboarding, so an athlete can import while still at the connect step.
  if (window.location.pathname === IMPORT_PATH) {
    return (
      <div className="min-h-dvh bg-surface">
        <OfflineBanner />
        <ImpersonationBanner />
        <ImportWindow
          onClose={() => {
            // A window the app opened closes; one reached in place returns.
            window.close();
            window.location.assign('/');
          }}
        />
      </div>
    );
  }

  // Authenticated and active — hand off to the onboarding flow, which renders
  // the current onboarding step (the first applicable step not yet complete) or
  // the dashboard once onboarding is finished. The step decision and its state
  // live in `useOnboardingState` and the step registry (`@pierre/shared-constants/onboarding`).
  return (
    <div className="min-h-dvh bg-surface">
      <OfflineBanner />
      <InstallAppBanner />
      <ImpersonationBanner />
      <OnboardingFlow
        state={onboarding}
        userDisplayName={user?.display_name}
        fallback={
          <UnitsProvider>
            <Dashboard
              // A mode switch redraws the shell from its default route rather
              // than carrying one mode's tab into the other.
              key={viewMode ?? 'athlete-account'}
              joinedRoom={heldInvite.joinedRoom}
              onJoinedRoomOpened={heldInvite.onJoinedRoomOpened}
            />
          </UnitsProvider>
        }
      />
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
              <ServiceWorkerUpdatePrompt />
            </ToastProvider>
          </AuthProvider>
        </ThemeProvider>
      </QueryClientProvider>
    </ErrorBoundary>
  );
}

export default App;
