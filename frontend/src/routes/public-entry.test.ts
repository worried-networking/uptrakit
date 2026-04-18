import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { page } from '$app/state';

vi.mock('$lib/api', () => ({
	getAuthMethods: vi.fn(),
	approveDeviceAuth: vi.fn()
}));

vi.mock('$lib/auth.svelte', () => ({
	getUser: vi.fn(() => null),
	getLoading: vi.fn(() => false),
	handleLogin: vi.fn(),
	handleRegister: vi.fn(),
	handleOidcLogin: vi.fn(),
	handleOidcCallback: vi.fn(),
	handleOidcLink: vi.fn(),
	handleOidcCompleteRegistration: vi.fn()
}));

vi.mock('$lib/stores/network.svelte', () => ({
	getIsOnline: vi.fn(() => true)
}));

import * as api from '$lib/api';
import {
	PUBLIC_ENTRY_INPUT_CLASS,
	PUBLIC_ENTRY_PRIMARY_BUTTON_CLASS
} from '$lib/components/ui/PublicEntryShell.svelte';
import LoginPage from './login/+page.svelte';
import RegisterPage from './register/+page.svelte';
import DevicePage from './device/+page.svelte';
import PublicErrorPage from './+error.svelte';

describe('Public entry shell contract', () => {
	beforeEach(() => {
		vi.clearAllMocks();
		page.url = new URL('http://localhost/login') as typeof page.url;
		(page as unknown as { status?: number; error?: { message: string } }).status = 500;
		(page as unknown as { status?: number; error?: { message: string } }).error = {
			message: 'Something broke'
		};

		vi.mocked(api.getAuthMethods).mockResolvedValue({
			password: true,
			oidc_providers: [],
			setup_required: false,
			registration_token_required: false
		});
	});

	it('renders login inside shared shell and shows inline required errors', async () => {
		render(LoginPage);

		await waitFor(() => expect(screen.getByRole('heading', { name: 'Login' })).toBeInTheDocument());
		expect(document.querySelector('[data-ui="public-entry-shell"]')).toBeInTheDocument();
		expect(screen.getByText('Use your account credentials or an identity provider.')).toBeInTheDocument();
		expect(document.querySelectorAll('[data-ui="form-field-row"]')).toHaveLength(2);
		expect(screen.getByRole('link', { name: 'Register' })).toHaveAttribute('href', '/register');

		const loginButton = screen.getByRole('button', { name: 'Login' });
		expect(loginButton.className).toContain(PUBLIC_ENTRY_PRIMARY_BUTTON_CLASS);
		expect(loginButton.className).toContain('focus-visible:shadow');
		const loginForm = loginButton.closest('form');
		expect(loginForm).not.toBeNull();
		await fireEvent.submit(loginForm!);

		expect(screen.getByText('Email is required.')).toBeInTheDocument();
		expect(screen.getByText('Password is required.')).toBeInTheDocument();
		expect(screen.getByLabelText('Email')).toHaveAttribute('aria-invalid', 'true');
		expect(screen.getByLabelText('Password')).toHaveAttribute('aria-invalid', 'true');
		expect(screen.getByLabelText('Email').className).toContain(PUBLIC_ENTRY_INPUT_CLASS);
	});

	it('renders register inside shared shell and validates required fields inline', async () => {
		page.url = new URL('http://localhost/register') as typeof page.url;

		render(RegisterPage);

		expect(screen.getByRole('heading', { name: 'Register' })).toBeInTheDocument();
		expect(document.querySelector('[data-ui="public-entry-shell"]')).toBeInTheDocument();
		expect(screen.getByText('Create your local account to sign in later.')).toBeInTheDocument();
		expect(document.querySelectorAll('[data-ui="form-field-row"]')).toHaveLength(4);
		expect(screen.getByRole('link', { name: 'Login' })).toHaveAttribute('href', '/login');

		const registerButton = screen.getByRole('button', { name: 'Register' });
		expect(registerButton.className).toContain(PUBLIC_ENTRY_PRIMARY_BUTTON_CLASS);
		expect(registerButton.className).toContain('focus-visible:shadow');

		const registerForm = registerButton.closest('form');
		expect(registerForm).not.toBeNull();
		await fireEvent.submit(registerForm!);

		expect(screen.getByText('Email is required.')).toBeInTheDocument();
		expect(screen.getByText('First name is required.')).toBeInTheDocument();
		expect(screen.getByText('Last name is required.')).toBeInTheDocument();
		expect(screen.getByText('Password is required.')).toBeInTheDocument();
		expect(screen.getByLabelText('Email')).toHaveAttribute('aria-invalid', 'true');
		expect(screen.getByLabelText('Password')).toHaveAttribute('aria-invalid', 'true');
	});

	it('uses semantic invalid-code callout treatment on /device', () => {
		page.url = new URL('http://localhost/device?code=AB12-1BAD') as typeof page.url;

		render(DevicePage);

		expect(document.querySelector('[data-ui="public-entry-shell"]')).toBeInTheDocument();
		const invalidCodeCallout = document.querySelector('[data-ui="callout"][data-tone="danger"]');
		expect(invalidCodeCallout).toBeInTheDocument();
		expect(invalidCodeCallout).toHaveTextContent('Invalid device code format');
		expect(screen.getByRole('heading', { name: 'Authorize Device' })).toBeInTheDocument();
	});

	it('renders +error with shared public shell framing', () => {
		render(PublicErrorPage);

		expect(document.querySelector('[data-ui="public-entry-shell"]')).toBeInTheDocument();
		const errorCallout = document.querySelector('[data-ui="callout"][data-tone="danger"]');
		expect(errorCallout).toBeInTheDocument();
		expect(screen.getByRole('heading', { name: 'Something went wrong' })).toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Go to Home' })).toBeInTheDocument();
	});
});
