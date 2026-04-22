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
import * as auth from '$lib/auth.svelte';
import * as network from '$lib/stores/network.svelte';
import { PUBLIC_ENTRY_INPUT_CLASS } from '$lib/components/ui/PublicEntryShell.svelte';
import LoginPage from './login/+page.svelte';
import RegisterPage from './register/+page.svelte';
import DevicePage from './device/+page.svelte';
import PublicErrorPage from './+error.svelte';

const PRIMARY_GRADIENT = 'bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]';
const BUTTON_HEIGHT = 'h-[23px]';
const GHOST_BG = 'bg-transparent';

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
		expect(loginButton.className).toContain(BUTTON_HEIGHT);
		expect(loginButton.className).toContain(PRIMARY_GRADIENT);
		expect(loginButton.getAttribute('type')).toBe('submit');
		expect(loginButton.getAttribute('aria-busy')).toBeNull();
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
		expect(screen.getByRole('button', { name: 'Login' })).toHaveAttribute('href', '/login');

		const registerButton = screen.getByRole('button', { name: 'Register' });
		expect(registerButton.className).toContain(BUTTON_HEIGHT);
		expect(registerButton.className).toContain(PRIMARY_GRADIENT);
		expect(registerButton.getAttribute('type')).toBe('submit');
		expect(registerButton.getAttribute('aria-busy')).toBeNull();

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
		const goHomeLink = document.querySelector('a[href="/"]');
		expect(goHomeLink).toBeInTheDocument();
		expect(goHomeLink!.className).toContain(BUTTON_HEIGHT);
		expect(goHomeLink!.className).toContain(PRIMARY_GRADIENT);
	});

	it('OIDC button shows aria-busy=true while loading', async () => {
		vi.mocked(api.getAuthMethods).mockResolvedValue({
			password: false,
			oidc_providers: [{ id: 'google', name: 'Google', slug: 'google' }],
			setup_required: false,
			registration_token_required: false
		});
		vi.mocked(auth.handleOidcLogin).mockReturnValue(new Promise(() => {}));

		render(LoginPage);

		await waitFor(() => expect(screen.getByRole('button', { name: 'Login with Google' })).toBeInTheDocument());

		const oidcBtn = screen.getByRole('button', { name: 'Login with Google' });
		expect(oidcBtn.className).toContain(BUTTON_HEIGHT);
		expect(oidcBtn.className).toContain(GHOST_BG);
		expect(oidcBtn.getAttribute('aria-busy')).toBeNull();

		await fireEvent.click(oidcBtn);

		await waitFor(() => expect(oidcBtn.getAttribute('aria-busy')).toBe('true'));
		expect(screen.queryByText('Redirecting...')).not.toBeInTheDocument();
	});

	it('Approve button shows aria-busy=true while approving', async () => {
		page.url = new URL('http://localhost/device?code=BCDF-GHJK') as typeof page.url;
		vi.mocked(auth.getUser).mockReturnValue({
			id: '1',
			email: 'user@example.com',
			first_name: 'User',
			last_name: 'Test',
			permissions: []
		});
		vi.mocked(api.approveDeviceAuth).mockReturnValue(new Promise(() => {}));

		render(DevicePage);

		const approveBtn = screen.getByRole('button', { name: 'Approve' });
		expect(approveBtn.className).toContain(BUTTON_HEIGHT);
		expect(approveBtn.className).toContain(PRIMARY_GRADIENT);
		expect(approveBtn.getAttribute('aria-busy')).toBeNull();

		await fireEvent.click(approveBtn);

		await waitFor(() => expect(approveBtn.getAttribute('aria-busy')).toBe('true'));
		expect(screen.queryByText('Authorizing...')).not.toBeInTheDocument();
	});

	it('submit buttons are disabled when offline', async () => {
		vi.mocked(network.getIsOnline).mockReturnValue(false);

		render(LoginPage);

		await waitFor(() => expect(screen.getByRole('heading', { name: 'Login' })).toBeInTheDocument());

		const loginButton = screen.getByRole('button', { name: 'Login' });
		expect(loginButton).toBeDisabled();
	});

	it('register submit button is disabled when offline', async () => {
		page.url = new URL('http://localhost/register') as typeof page.url;
		vi.mocked(network.getIsOnline).mockReturnValue(false);

		render(RegisterPage);

		const registerButton = screen.getByRole('button', { name: 'Register' });
		expect(registerButton).toBeDisabled();
	});

	it('+error Go to Home renders as an anchor href link to /', () => {
		render(PublicErrorPage);

		const goHomeLink = document.querySelector('a[href="/"]');
		expect(goHomeLink).toBeInTheDocument();
		expect(goHomeLink!.getAttribute('role')).toBe('button');
	});
});
