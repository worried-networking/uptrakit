import { afterEach, describe, expect, it } from 'vitest';
import { portal } from './portal';

afterEach(() => {
	document.body.querySelectorAll('[data-test-portal]').forEach((node) => node.remove());
});

describe('portal action', () => {
	it('reparents the host node to document.body on mount', () => {
		const wrapper = document.createElement('div');
		const node = document.createElement('div');
		node.setAttribute('data-test-portal', '');
		wrapper.appendChild(node);

		expect(node.parentElement).toBe(wrapper);

		portal(node);

		expect(node.parentElement).toBe(document.body);
		expect(wrapper.contains(node)).toBe(false);
	});

	it('removes the host node from document.body on destroy', () => {
		const node = document.createElement('div');
		node.setAttribute('data-test-portal', '');

		const handle = portal(node);
		expect(document.body.contains(node)).toBe(true);

		handle.destroy();
		expect(document.body.contains(node)).toBe(false);
	});

	it('destroy is safe when the node has already been detached', () => {
		const node = document.createElement('div');
		node.setAttribute('data-test-portal', '');

		const handle = portal(node);
		node.remove();

		expect(() => handle.destroy()).not.toThrow();
	});
});
