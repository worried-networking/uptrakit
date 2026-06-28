// Ported verbatim from frontend/src/lib/api.ts lines 109, 165-215.
// api.ts itself stays UNTOUCHED; this module is reused by client.ts (Task 4)
// and, in Plan C, by all call sites.

export const MAX_ERROR_LENGTH = 500;

export function truncateError(msg: string): string {
	return msg.length > MAX_ERROR_LENGTH ? msg.slice(0, MAX_ERROR_LENGTH) + '…' : msg;
}

export async function extractErrorMessage(res: Response): Promise<string> {
	const text = await res.text();
	if (!text) return res.statusText;
	try {
		const parsed = JSON.parse(text);
		if (typeof parsed === 'object' && parsed !== null && typeof parsed.error === 'string') {
			return truncateError(parsed.error);
		}
	} catch {
		/* Not JSON */
	}
	return truncateError(text);
}

export class ApiError extends Error {
	public readonly errorCode: string | null;
	public readonly status: number;

	constructor(message: string, status: number, errorCode: string | null) {
		super(message);
		this.name = 'ApiError';
		this.status = status;
		this.errorCode = errorCode;
	}
}

export async function extractApiError(res: Response): Promise<ApiError> {
	const text = await res.text();
	let message: string = res.statusText;
	let errorCode: string | null = null;
	if (text) {
		try {
			const parsed = JSON.parse(text);
			if (typeof parsed === 'object' && parsed !== null) {
				if (typeof parsed.error === 'string') {
					message = truncateError(parsed.error);
				}
				if (typeof parsed.error_code === 'string') {
					errorCode = parsed.error_code;
				}
			}
		} catch {
			message = truncateError(text);
		}
	}
	return new ApiError(message, res.status, errorCode);
}
