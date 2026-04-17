import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import TabStrip from './TabStrip.svelte';

afterEach(() => {
	cleanup();
});

describe('TabStrip', () => {
	it('marks the active tab with a semantic state attribute', () => {
		render(TabStrip, {
			items: [
				{ id: 'general', label: 'General', panelId: 'panel-general' },
				{ id: 'plugin-configs', label: 'Plugin Configs', panelId: 'panel-plugin-configs' }
			],
			activeId: 'general',
			idBase: 'settings'
		});

		expect(screen.getByRole('tab', { name: 'General' })).toHaveAttribute('data-state', 'active');
		expect(screen.getByRole('tab', { name: 'General' })).toHaveAttribute('aria-controls', 'panel-general');
		expect(screen.getByRole('tab', { name: 'General' })).toHaveAttribute('id', 'settings-tab-general');
		expect(screen.getByRole('tab', { name: 'Plugin Configs' })).toHaveAttribute('data-state', 'inactive');
	});

	it('supports keyboard navigation across enabled tabs', async () => {
		const selected: string[] = [];
		render(TabStrip, {
			items: [
				{ id: 'general', label: 'General', panelId: 'panel-general' },
				{ id: 'advanced', label: 'Advanced', panelId: 'panel-advanced', disabled: true },
				{ id: 'plugin-configs', label: 'Plugin Configs', panelId: 'panel-plugin-configs' }
			],
			activeId: 'general',
			onSelect(id) {
				selected.push(id);
			}
		});

		const generalTab = screen.getByRole('tab', { name: 'General' });
		const pluginConfigsTab = screen.getByRole('tab', { name: 'Plugin Configs' });
		generalTab.focus();

		await fireEvent.keyDown(generalTab, { key: 'ArrowRight' });

		expect(selected).toEqual(['plugin-configs']);
		expect(pluginConfigsTab).toHaveFocus();
	});
});
