import { describe, expect, it } from 'vitest';
import { load } from './[id]/+page';

describe('/extensions/[id] compatibility redirect', () => {
	it('redirects to the canonical /surfaces/[id] route', () => {
		try {
			load({
				params: { id: 'surface.one' },
				url: new URL('http://localhost/extensions/surface.one')
			} as never);
		} catch (error) {
			expect(error).toMatchObject({
				status: 307,
				location: '/surfaces/surface.one'
			});
			return;
		}

		throw new Error('Expected redirect to be thrown');
	});

	it('preserves search params when redirecting to /surfaces/[id]', () => {
		try {
			load({
				params: { id: 'surface.one' },
				url: new URL('http://localhost/extensions/surface.one?from=legacy&tab=1')
			} as never);
		} catch (error) {
			expect(error).toMatchObject({
				status: 307,
				location: '/surfaces/surface.one?from=legacy&tab=1'
			});
			return;
		}

		throw new Error('Expected redirect to be thrown');
	});
});
