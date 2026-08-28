// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, vi } from 'vitest'
import { render, screen, waitFor, act } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import Login from '../Login'
import { AuthProvider } from '../../contexts/AuthContext'
import { ThemeProvider } from '../../hooks/useTheme'

// vi.hoisted runs before vi.mock hoisting, so these variables are available in the factory
const { mockAuthStorage } = vi.hoisted(() => ({
  mockAuthStorage: {
    setCsrfToken: vi.fn().mockResolvedValue(undefined),
    getCsrfToken: vi.fn().mockResolvedValue(null),
    setUser: vi.fn().mockResolvedValue(undefined),
    getUser: vi.fn().mockResolvedValue(null),
    clear: vi.fn().mockResolvedValue(undefined),
    getToken: vi.fn().mockResolvedValue(null),
    setToken: vi.fn().mockResolvedValue(undefined),
    removeToken: vi.fn().mockResolvedValue(undefined),
    getRefreshToken: vi.fn().mockResolvedValue(null),
    setRefreshToken: vi.fn().mockResolvedValue(undefined),
  },
}))

// Mock the API service - AuthContext uses authApi, pierreApi, adminApi
vi.mock('../../services/api', () => ({
  authApi: {
    login: vi.fn(),
    logout: vi.fn().mockResolvedValue(undefined),
  },
  adminApi: {
    getSetupStatus: vi.fn().mockResolvedValue({ needs_setup: false, admin_exists: true }),
    endImpersonation: vi.fn(),
  },
  pierreApi: {
    adapter: {
      authStorage: mockAuthStorage,
    },
  },
}))

async function renderLogin(props: { prefilledEmail?: string } = {}) {
  let result;
  await act(async () => {
    result = render(
      <ThemeProvider>
        <AuthProvider>
          <Login prefilledEmail={props.prefilledEmail} />
        </AuthProvider>
      </ThemeProvider>
    );
    // Wait for setup status check to complete
    await waitFor(() => {
      expect(screen.queryByText('Checking setup...')).not.toBeInTheDocument();
    }, { timeout: 1000 });
  });
  return result;
}

describe('Login Component', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('should render login form', async () => {
    await renderLogin()

    expect(screen.getByRole('heading', { name: /sign in/i })).toBeInTheDocument()
    expect(screen.getByLabelText(/email address/i)).toBeInTheDocument()
    expect(screen.getByLabelText(/^password$/i)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /sign in/i })).toBeInTheDocument()
  })

  it('should allow user to type in email and password fields', async () => {
    const user = userEvent.setup()
    await renderLogin()

    const emailInput = screen.getByLabelText(/email address/i)
    const passwordInput = screen.getByLabelText(/^password$/i)

    await user.type(emailInput, 'test@example.com')
    await user.type(passwordInput, 'password123')

    expect(emailInput).toHaveValue('test@example.com')
    expect(passwordInput).toHaveValue('password123')
  })

  it('should require email and password fields', async () => {
    const user = userEvent.setup()
    await renderLogin()

    const submitButton = screen.getByRole('button', { name: /sign in/i })

    // Try to submit without filling fields
    await user.click(submitButton)

    // HTML5 validation should prevent submission
    expect(screen.getByLabelText(/email address/i)).toBeRequired()
    expect(screen.getByLabelText(/^password$/i)).toBeRequired()
  })

  it('should show loading state during login', async () => {
    const user = userEvent.setup()
    const { authApi } = await import('../../services/api')

    // Make login hang to test loading state
    vi.mocked(authApi.login).mockImplementation(() => new Promise(() => {}))

    await renderLogin()

    const emailInput = screen.getByLabelText(/email address/i)
    const passwordInput = screen.getByLabelText(/^password$/i)
    const submitButton = screen.getByRole('button', { name: /sign in/i })

    await user.type(emailInput, 'test@example.com')
    await user.type(passwordInput, 'password123')
    await user.click(submitButton)

    expect(screen.getByText(/signing in/i)).toBeInTheDocument()
    expect(submitButton).toBeDisabled()
  })

  it('should display error message on login failure', async () => {
    const user = userEvent.setup()
    const { authApi } = await import('../../services/api')

    // A real axios rejection always carries the status alongside the body;
    // the classifier reads the status, so the fixture has to have one.
    const mockError = {
      response: {
        status: 401,
        data: {
          error: 'Invalid credentials'
        }
      }
    }

    vi.mocked(authApi.login).mockRejectedValue(mockError)

    await renderLogin()

    const emailInput = screen.getByLabelText(/email address/i)
    const passwordInput = screen.getByLabelText(/^password$/i)
    const submitButton = screen.getByRole('button', { name: /sign in/i })

    await user.type(emailInput, 'test@example.com')
    await user.type(passwordInput, 'wrongpassword')
    await user.click(submitButton)

    await waitFor(() => {
      // Login component maps "Invalid credentials" to user-friendly message
      expect(screen.getByText('Invalid email or password')).toBeInTheDocument()
    })

    // Should not be loading anymore
    expect(screen.getByRole('button', { name: /sign in/i })).toBeInTheDocument()
    expect(submitButton).not.toBeDisabled()
  })

  /** Fill the form and submit it, returning once the request has been made. */
  async function submitCredentials() {
    const user = userEvent.setup()
    await renderLogin()
    await user.type(screen.getByLabelText(/email address/i), 'test@example.com')
    await user.type(screen.getByLabelText(/^password$/i), 'password123')
    await user.click(screen.getByRole('button', { name: /sign in/i }))
  }

  /** Pin navigator.onLine for one test; jsdom reports true by default. */
  function setOnline(value: boolean) {
    Object.defineProperty(window.navigator, 'onLine', {
      configurable: true,
      get: () => value,
    })
  }

  it('reads a reachable-but-failing network as a network error, not a bad password', async () => {
    const { authApi } = await import('../../services/api')
    vi.mocked(authApi.login).mockRejectedValue(new Error('Network error'))
    setOnline(true)

    await submitCredentials()

    await waitFor(() => {
      expect(screen.getByText('Network error. Check your connection.')).toBeInTheDocument()
    })
    // The defect this replaced: every non-credential failure read as one.
    expect(screen.queryByText('Invalid email or password')).not.toBeInTheDocument()
  })

  it('tells an OFFLINE athlete they are offline instead of blaming their password', async () => {
    const { authApi } = await import('../../services/api')
    // A request that never reached a server: no `response` on the rejection.
    vi.mocked(authApi.login).mockRejectedValue(new Error('Network Error'))
    setOnline(false)

    await submitCredentials()

    await waitFor(() => {
      expect(
        screen.getByText("You're offline. Check your connection and try again."),
      ).toBeInTheDocument()
    })
    expect(screen.queryByText('Invalid email or password')).not.toBeInTheDocument()
    setOnline(true)
  })

  it('still names a genuinely rejected credential, in any server language', async () => {
    const { authApi } = await import('../../services/api')
    // A French backend: the old code matched on the substring "Invalid" and
    // would have fallen through to the generic failure here.
    vi.mocked(authApi.login).mockRejectedValue({
      response: { status: 401, data: { error: 'Identifiants invalides' } },
    })
    setOnline(true)

    await submitCredentials()

    await waitFor(() => {
      expect(screen.getByText('Invalid email or password')).toBeInTheDocument()
    })
  })

  it('should prefill the email field when prefilledEmail prop is supplied', async () => {
    await renderLogin({ prefilledEmail: 'alice@acme.com' })

    const emailInput = screen.getByLabelText(/email address/i)
    expect(emailInput).toHaveValue('alice@acme.com')
  })

  it('should leave the email field empty when no prefilledEmail is supplied', async () => {
    await renderLogin()

    const emailInput = screen.getByLabelText(/email address/i)
    expect(emailInput).toHaveValue('')
  })

  it('should use a synthetic placeholder that cannot be mistaken for a pre-filled value', async () => {
    await renderLogin()

    const emailInput = screen.getByLabelText(/email address/i)
    // RFC 2606 reserves example.com for documentation; this prevents
    // users from thinking their own email has already been entered.
    expect(emailInput).toHaveAttribute('placeholder', 'name@example.com')
  })

  it('should have proper accessibility attributes', async () => {
    await renderLogin()

    const emailInput = screen.getByLabelText(/email address/i)
    const passwordInput = screen.getByLabelText(/^password$/i)

    expect(emailInput).toHaveAttribute('type', 'email')
    expect(emailInput).toHaveAttribute('required')
    expect(passwordInput).toHaveAttribute('type', 'password')
    expect(passwordInput).toHaveAttribute('required')
  })
})
