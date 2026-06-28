import { describe, it, expect } from 'vitest';
import { ApiError, extractApiError, extractErrorMessage } from './errors';

describe('errors', () => {
	it('extractApiError maps JSON error + error_code + status', async () => {
		const res = new Response(JSON.stringify({ error: 'nope', error_code: 'bad_thing' }), { status: 409 });
		const e = await extractApiError(res);
		expect(e).toBeInstanceOf(ApiError);
		expect(e.status).toBe(409);
		expect(e.errorCode).toBe('bad_thing');
		expect(e.message).toBe('nope');
	});

	it('extractApiError falls back to statusText / raw text', async () => {
		const res = new Response('boom', {
			status: 500,
			statusText: 'Internal Server Error'
		});
		const e = await extractApiError(res);
		expect(e.status).toBe(500);
		expect(e.errorCode).toBeNull();
		expect(e.message).toBe('boom');
	});

	it('extractErrorMessage truncates very long bodies', async () => {
		const res = new Response('x'.repeat(900), { status: 400 });
		const msg = await extractErrorMessage(res);
		expect(msg.length).toBeLessThanOrEqual(501); // 500 + ellipsis
	});
});
