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

	// Ported from the deleted src/lib/api.test.ts (extractErrorMessage edge cases).

	it('extractErrorMessage returns the full JSON string when no error field is present', async () => {
		const body = JSON.stringify({ message: 'something went wrong' });
		const res = new Response(body, { status: 500, statusText: 'Internal Server Error' });
		const msg = await extractErrorMessage(res);
		expect(msg).toBe(body);
	});

	it('extractErrorMessage returns statusText when body is empty', async () => {
		const res = new Response('', { status: 401, statusText: 'Unauthorized' });
		const msg = await extractErrorMessage(res);
		expect(msg).toBe('Unauthorized');
	});

	it('extractErrorMessage ignores non-string error fields in JSON', async () => {
		const body = JSON.stringify({ error: 42 });
		const res = new Response(body, { status: 400, statusText: 'Bad Request' });
		const msg = await extractErrorMessage(res);
		// error is not a string, so falls back to the full JSON body
		expect(msg).toBe(body);
	});

	it('extractErrorMessage truncates a JSON error field longer than 500 characters', async () => {
		const longError = 'e'.repeat(600);
		const res = new Response(JSON.stringify({ error: longError }), { status: 400, statusText: 'Bad Request' });
		const msg = await extractErrorMessage(res);
		expect(msg).toHaveLength(501);
		expect(msg.endsWith('…')).toBe(true);
	});

	it('extractErrorMessage does not truncate messages of exactly 500 characters', async () => {
		const exactMessage = 'b'.repeat(500);
		const res = new Response(exactMessage, { status: 500, statusText: 'Internal Server Error' });
		const msg = await extractErrorMessage(res);
		expect(msg).toBe(exactMessage);
	});
});
