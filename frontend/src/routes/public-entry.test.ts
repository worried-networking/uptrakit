import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
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
import Checkbox from '$lib/components/Checkbox.svelte';
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

		const emailInput = screen.getByLabelText('Email');
		expect(emailInput.getAttribute('id')).toBe('login-email');
		expect(emailInput.getAttribute('type')).toBe('email');
		expect(emailInput.getAttribute('autocomplete')).toBe('email');

		const passwordInput = screen.getByLabelText('Password');
		expect(passwordInput.getAttribute('id')).toBe('login-password');
		expect(passwordInput.getAttribute('type')).toBe('password');
		expect(passwordInput.getAttribute('autocomplete')).toBe('current-password');
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

	it('text-swap guard: "Redirecting..." literal does not appear in login page', async () => {
		render(LoginPage);
		expect(document.body.textContent).not.toContain('Redirecting...');
		// Click an OIDC button without mocking (sync rejection is fine, just checking static render)
		expect(document.body.textContent).not.toContain('Redirecting...');
	});

	it('text-swap guard: "Authorizing..." literal does not appear in device page', () => {
		page.url = new URL('http://localhost/device?code=BCDF-GHJK') as typeof page.url;
		render(DevicePage);
		expect(document.body.textContent).not.toContain('Authorizing...');
	});

	it('OIDC button is disabled AND aria-busy=true when offline and loading simultaneously', async () => {
		vi.mocked(api.getAuthMethods).mockResolvedValue({
			password: false,
			oidc_providers: [{ id: 'google', name: 'Google', slug: 'google' }],
			setup_required: false,
			registration_token_required: false
		});
		vi.mocked(network.getIsOnline).mockReturnValue(false);
		vi.mocked(auth.handleOidcLogin).mockReturnValue(new Promise(() => {}));

		render(LoginPage);

		await waitFor(() => expect(screen.getByRole('button', { name: 'Login with Google' })).toBeInTheDocument());

		const oidcBtn = screen.getByRole('button', { name: 'Login with Google' });

		await fireEvent.click(oidcBtn);

		await waitFor(() => expect(oidcBtn.getAttribute('aria-busy')).toBe('true'));
		expect(oidcBtn).toBeDisabled();
	});

	it('login-email Input aria-describedby points to FormFieldRow error copy id after error set', async () => {
		render(LoginPage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Login' })).toBeInTheDocument());

		const loginForm = screen.getByRole('button', { name: 'Login' }).closest('form');
		await fireEvent.submit(loginForm!);

		await waitFor(() => expect(screen.getByLabelText('Email')).toHaveAttribute('aria-invalid', 'true'));

		const emailInput = screen.getByLabelText('Email');
		const describedById = emailInput.getAttribute('aria-describedby');
		expect(describedById).toBeTruthy();
		const errorNode = document.getElementById(describedById!);
		expect(errorNode).not.toBeNull();
		expect(errorNode!.textContent?.trim().length).toBeGreaterThan(0);
	});

	it('registration-token Input renders type=text and autocomplete=off under registrationTokenRequired', async () => {
		page.url = new URL(
			'http://localhost/login#registration_token_required=true&registration_code=RC123'
		) as typeof page.url;

		render(LoginPage);

		await waitFor(() => expect(screen.getByLabelText('Registration token')).toBeInTheDocument());

		const tokenInput = screen.getByLabelText('Registration token');
		expect(tokenInput.getAttribute('id')).toBe('registration-token');
		expect(tokenInput.getAttribute('type')).toBe('text');
		expect(tokenInput.getAttribute('autocomplete')).toBe('off');
	});

	it('link-password Input renders type=password and autocomplete=current-password under linkRequired', async () => {
		page.url = new URL('http://localhost/login?link_required=true&email=user@example.com') as typeof page.url;

		render(LoginPage);

		await waitFor(() => expect(screen.getByLabelText('Password')).toBeInTheDocument());

		const linkPwInput = screen.getByLabelText('Password');
		expect(linkPwInput.getAttribute('id')).toBe('link-password');
		expect(linkPwInput.getAttribute('type')).toBe('password');
		expect(linkPwInput.getAttribute('autocomplete')).toBe('current-password');
	});

	it('Checkbox renders opacity-40 class when disabled=true', () => {
		const { container } = render(Checkbox, { id: 'test-cb', checked: false, disabled: true });
		const checkbox = container.querySelector('#test-cb') as HTMLInputElement;
		expect(checkbox).not.toBeNull();
		const wrapper = checkbox.closest('[class*="opacity"]') ?? checkbox.parentElement;
		expect(wrapper?.className ?? checkbox.className).toContain('opacity-40');
	});

	it('show-token Checkbox renders with id=show-token, toggles field, and fires onchange exactly once per click', async () => {
		page.url = new URL('http://localhost/register') as typeof page.url;

		render(RegisterPage);

		const checkbox = document.querySelector('#show-token') as HTMLInputElement;
		expect(checkbox).not.toBeNull();
		expect(checkbox.getAttribute('type')).toBe('checkbox');
		expect(checkbox.checked).toBe(false);

		const handler = vi.fn();
		checkbox.addEventListener('change', handler);

		await fireEvent.click(checkbox);

		expect(handler).toHaveBeenCalledTimes(1);
		expect(checkbox.checked).toBe(true);
		await waitFor(() => expect(screen.getByLabelText('Invite token')).toBeInTheDocument());

		await fireEvent.click(checkbox);
		expect(handler).toHaveBeenCalledTimes(2);
		await waitFor(() => expect(screen.queryByLabelText('Invite token')).not.toBeInTheDocument());
	});

	it('login footer Register link renders as <Link> with href=/register', async () => {
		render(LoginPage);

		await waitFor(() => expect(screen.getByRole('heading', { name: 'Login' })).toBeInTheDocument());

		const registerLink = screen.getByRole('link', { name: 'Register' });
		expect(registerLink).toHaveAttribute('href', '/register');
		expect(registerLink.className).toContain('font-medium');
		expect(registerLink.className).toContain('underline');
	});

	it('regression: deleted PUBLIC_ENTRY_INPUT/CHECKBOX/LINK_CLASS literal strings absent from DOM', async () => {
		render(LoginPage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Login' })).toBeInTheDocument());
		// Input class fragment
		expect(document.body.innerHTML).not.toContain('rounded-lg border border-[var(--border-default)]');
		// Link class fragment
		expect(document.body.innerHTML).not.toContain('hover:text-[var(--accent-bright)] focus-visible:outline-none');

		// Checkbox class fragment — needs register page
		cleanup();
		page.url = new URL('http://localhost/register') as typeof page.url;
		render(RegisterPage);
		await waitFor(() => expect(screen.getByRole('heading', { name: 'Register' })).toBeInTheDocument());
		expect(document.body.innerHTML).not.toContain('checkbox h-4 w-4 rounded border-[var(--border-default)]');
	});
});
