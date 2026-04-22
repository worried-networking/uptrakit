import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import ConfirmDialog from './ConfirmDialog.svelte';

afterEach(() => {
	cleanup();
	vi.clearAllMocks();
});

const defaultProps = {
	title: 'Delete Item',
	messagePrefix: 'Are you sure you want to permanently delete',
	entityName: 'my-service',
	confirmLabel: 'Delete',
	onconfirm: vi.fn(),
	oncancel: vi.fn()
};

describe('ConfirmDialog', () => {
	it('renders the dialog title', () => {
		const { container } = render(ConfirmDialog, defaultProps);
		expect(screen.getByRole('dialog')).toBeInTheDocument();
		expect(screen.getByText('Delete Item')).toBeInTheDocument();
		expect(container.querySelector('[data-ui="section-card"]')).toBeInTheDocument();
		expect(container.querySelector('[data-ui="status-badge"]')).toBeInTheDocument();
	});

	it('renders the message with the entity name', () => {
		render(ConfirmDialog, defaultProps);
		expect(screen.getByText(/Are you sure you want to permanently delete/)).toBeInTheDocument();
		expect(screen.getByText('my-service')).toBeInTheDocument();
	});

	it('renders the confirm button with the given label', () => {
		render(ConfirmDialog, defaultProps);
		expect(screen.getByRole('button', { name: 'Delete' })).toBeInTheDocument();
	});

	it('calls onconfirm when the confirm button is clicked', () => {
		const onconfirm = vi.fn();
		render(ConfirmDialog, { ...defaultProps, onconfirm });
		fireEvent.click(screen.getByRole('button', { name: 'Delete' }));
		expect(onconfirm).toHaveBeenCalledOnce();
	});

	it('calls oncancel when the Cancel button is clicked', () => {
		const oncancel = vi.fn();
		render(ConfirmDialog, { ...defaultProps, oncancel });
		fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
		expect(oncancel).toHaveBeenCalledOnce();
	});

	it('disables the confirm button when confirmDisabled is true', () => {
		render(ConfirmDialog, { ...defaultProps, confirmDisabled: true });
		expect(screen.getByRole('button', { name: 'Delete' })).toBeDisabled();
	});

	it('uses the custom confirm button label', () => {
		render(ConfirmDialog, { ...defaultProps, confirmLabel: 'Processing...' });
		expect(screen.getByRole('button', { name: 'Processing...' })).toBeInTheDocument();
	});

	it('renders warnings using the shared callout primitive', () => {
		const { container } = render(ConfirmDialog, {
			...defaultProps,
			warnings: ['This action cannot be undone']
		});
		expect(screen.getByText('This action cannot be undone')).toBeInTheDocument();
		expect(container.querySelector('[data-ui="callout"][data-tone="warning"]')).toBeInTheDocument();
	});

	it('confirm button renders variant="danger" by default', () => {
		render(ConfirmDialog, defaultProps);
		const confirmBtn = screen.getByRole('button', { name: 'Delete' });
		expect(confirmBtn.className).toMatch(/danger|error/);
	});

	it('confirm button renders variant="primary" when confirmVariant="primary"', () => {
		render(ConfirmDialog, { ...defaultProps, confirmVariant: 'primary' });
		const confirmBtn = screen.getByRole('button', { name: 'Delete' });
		expect(confirmBtn.className).not.toMatch(/danger|error/);
	});

	it('cancel button renders variant="secondary"', () => {
		render(ConfirmDialog, defaultProps);
		const cancelBtn = screen.getByRole('button', { name: 'Cancel' });
		expect(cancelBtn.className).toContain('border');
	});

	it('confirm button is NOT disabled when confirmDisabled=false (default)', () => {
		render(ConfirmDialog, defaultProps);
		expect(screen.getByRole('button', { name: 'Delete' })).not.toBeDisabled();
	});

	it('confirm button IS disabled when confirmDisabled=true', () => {
		render(ConfirmDialog, { ...defaultProps, confirmDisabled: true });
		expect(screen.getByRole('button', { name: 'Delete' })).toBeDisabled();
	});
});
