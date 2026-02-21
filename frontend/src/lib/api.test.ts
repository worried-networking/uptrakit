import { describe, expect, it, vi } from 'vitest';
import { extractErrorMessage } from './api';

// Mock ./auth to avoid Svelte store initialization in test environment
vi.mock('./auth', () => ({
	getAccessToken: vi.fn().mockReturnValue(null),
	setAccessToken: vi.fn()
}));

// ── extractErrorMessage ───────────────────────────────────────────────────────

describe('extractErrorMessage', () => {
	it('extracts the error field from a JSON body', async () => {
		const res = new Response(JSON.stringify({ error: 'Not found' }), {
			status: 404,
			statusText: 'Not Found'
		});
		const msg = await extractErrorMessage(res);
		expect(msg).toBe('Not found');
	});

	it('returns the full JSON string when no error field is present', async () => {
		const body = JSON.stringify({ message: 'something went wrong' });
		const res = new Response(body, { status: 500, statusText: 'Internal Server Error' });
		const msg = await extractErrorMessage(res);
		expect(msg).toBe(body);
	});

	it('returns plain text body for non-JSON responses', async () => {
		const res = new Response('Internal Server Error', {
			status: 500,
			statusText: 'Internal Server Error'
		});
		const msg = await extractErrorMessage(res);
		expect(msg).toBe('Internal Server Error');
	});

	it('returns statusText when body is empty', async () => {
		const res = new Response('', { status: 401, statusText: 'Unauthorized' });
		const msg = await extractErrorMessage(res);
		expect(msg).toBe('Unauthorized');
	});

	it('ignores non-string error fields in JSON', async () => {
		const body = JSON.stringify({ error: 42 });
		const res = new Response(body, { status: 400, statusText: 'Bad Request' });
		const msg = await extractErrorMessage(res);
		// error is not a string, so falls back to the full JSON body
		expect(msg).toBe(body);
	});
});
