import { describe, expect, it, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/svelte';
import RadioCardGroup from './RadioCardGroup.svelte';

const options = [
	{ value: 'open', label: 'Open', description: 'Anyone can create an account.' },
	{ value: 'invite', label: 'Invite Only', description: 'Token required.' },
	{ value: 'closed', label: 'Closed', description: 'No new accounts.' }
];

describe('RadioCardGroup', () => {
	it('renders all options', () => {
		render(RadioCardGroup, { name: 'mode', value: 'open', options });
		expect(screen.getByText('Open')).toBeTruthy();
		expect(screen.getByText('Invite Only')).toBeTruthy();
		expect(screen.getByText('Closed')).toBeTruthy();
	});

	it('selected card has aria-checked=true', () => {
		render(RadioCardGroup, { name: 'mode', value: 'invite', options });
		const inviteCard = screen.getByRole('radio', { name: /invite only/i });
		expect(inviteCard).toHaveAttribute('aria-checked', 'true');
	});

	it('unselected cards have aria-checked=false', () => {
		render(RadioCardGroup, { name: 'mode', value: 'open', options });
		const inviteCard = screen.getByRole('radio', { name: /invite only/i });
		expect(inviteCard).toHaveAttribute('aria-checked', 'false');
	});

	it('container has role=radiogroup', () => {
		render(RadioCardGroup, { name: 'mode', value: 'open', options });
		expect(screen.getByRole('radiogroup')).toBeTruthy();
	});

	it('calls onchange when a card is clicked', async () => {
		const onchange = vi.fn();
		render(RadioCardGroup, { name: 'mode', value: 'open', options, onchange });
		await fireEvent.click(screen.getByRole('radio', { name: /closed/i }));
		expect(onchange).toHaveBeenCalledWith('closed');
	});

	it('disabled cards do not fire onchange', async () => {
		const onchange = vi.fn();
		render(RadioCardGroup, { name: 'mode', value: 'open', options, onchange, disabled: true });
		await fireEvent.click(screen.getByRole('radio', { name: /closed/i }));
		expect(onchange).not.toHaveBeenCalled();
	});
});
