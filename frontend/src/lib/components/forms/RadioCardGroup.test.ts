import { describe, expect, it, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/svelte';
import RadioCardGroup from './RadioCardGroup.svelte';

const options = [
	{ value: 'open', label: 'Open', tooltip: 'Anyone can create an account.' },
	{ value: 'invite', label: 'Invite Only', tooltip: 'Token required.' },
	{ value: 'closed', label: 'Closed', tooltip: 'No new accounts.' }
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
		render(RadioCardGroup, {
			name: 'mode',
			value: 'open',
			options,
			onchange
		});
		await fireEvent.click(screen.getByRole('radio', { name: /closed/i }));
		expect(onchange).toHaveBeenCalledWith('closed');
	});

	it('disabled cards do not fire onchange', async () => {
		const onchange = vi.fn();
		render(RadioCardGroup, {
			name: 'mode',
			value: 'open',
			options,
			onchange,
			disabled: true
		});
		await fireEvent.click(screen.getByRole('radio', { name: /closed/i }));
		expect(onchange).not.toHaveBeenCalled();
	});

	it('renders info icon button when tooltip is set', () => {
		render(RadioCardGroup, { name: 'mode', value: 'open', options });
		const infoButtons = screen.getAllByRole('button', {
			name: 'More information'
		});
		expect(infoButtons.length).toBe(3);
	});

	it('renders no info icon button when tooltip is absent', () => {
		const optionsNoTooltip = [
			{ value: 'open', label: 'Open' },
			{ value: 'closed', label: 'Closed' }
		];
		render(RadioCardGroup, {
			name: 'mode',
			value: 'open',
			options: optionsNoTooltip
		});
		expect(screen.queryByRole('button', { name: 'More information' })).toBeNull();
	});

	it('clicking info icon does not select the card', async () => {
		const onchange = vi.fn();
		render(RadioCardGroup, {
			name: 'mode',
			value: 'open',
			options,
			onchange
		});
		const infoButton = screen.getAllByRole('button', {
			name: 'More information'
		})[2]; // closed card
		await fireEvent.click(infoButton);
		expect(onchange).not.toHaveBeenCalled();
	});

	it('Enter key on focused card fires onchange', async () => {
		const onchange = vi.fn();
		render(RadioCardGroup, { name: 'mode', value: 'open', options, onchange });
		const closedCard = screen.getByRole('radio', { name: /closed/i });
		await fireEvent.keyDown(closedCard, { key: 'Enter' });
		expect(onchange).toHaveBeenCalledWith('closed');
	});

	it('Space key on focused card fires onchange', async () => {
		const onchange = vi.fn();
		render(RadioCardGroup, { name: 'mode', value: 'open', options, onchange });
		const closedCard = screen.getByRole('radio', { name: /closed/i });
		await fireEvent.keyDown(closedCard, { key: ' ' });
		expect(onchange).toHaveBeenCalledWith('closed');
	});
});
