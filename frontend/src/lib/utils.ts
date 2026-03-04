/**
 * Validates that a URL is a safe logo URL (HTTPS only).
 * Returns true for valid https:// URLs, false for everything else
 * (including http://, javascript:, data:, and malformed URLs).
 */
export function isValidLogoUrl(url: string | null | undefined): boolean {
	if (!url) return false;
	try {
		const parsed = new URL(url);
		return parsed.protocol === 'https:';
	} catch {
		return false;
	}
}

/**
 * Formats a nullable ISO date string into a locale-appropriate string.
 * Returns an em-dash for null/undefined values.
 */
export function formatDate(date: string | null | undefined): string {
	if (!date) return '\u2014';
	return new Date(date).toLocaleString();
}

/**
 * Validates a redirect path: allows paths that start with "/" but not "//".
 * Rejects null, absolute URLs, and protocol-relative URLs to prevent open
 * redirect vulnerabilities. Returns "/" as the safe fallback.
 */
export function safeRedirect(redirect: string | null): string {
	if (redirect && redirect.startsWith('/') && !redirect.startsWith('//')) {
		return redirect;
	}
	return '/';
}

/**
 * Parse and validate an enum-typed URL search param.
 * Returns the fallback for missing or invalid values.
 */
export function parseUrlParam<T extends string>(url: URL, key: string, allowed: readonly T[], fallback: T): T {
	const val = url.searchParams.get(key);
	if (val !== null && (allowed as readonly string[]).includes(val)) {
		return val as T;
	}
	return fallback;
}

/**
 * Parse a page number from a URL search param.
 * Returns 1 for missing or invalid values (non-integer, zero, or negative).
 */
export function parseUrlPage(url: URL): number {
	const val = url.searchParams.get('page');
	if (val === null || !/^\d+$/.test(val)) return 1;
	const num = parseInt(val, 10);
	return num >= 1 ? num : 1;
}

/**
 * Formats a version string for display.
 * SHA-256 digests (sha256:...) are shortened to "sha256:<first 12 hex chars>…".
 * Returns the fallback string for null/undefined values.
 */
export function formatVersion(version: string | null | undefined, fallback = '—'): string {
	if (!version) return fallback;
	if (version.startsWith('sha256:')) {
		return `sha256:${version.slice(7, 19)}\u2026`;
	}
	return version;
}

/**
 * Copies text to the clipboard. Returns true on success, false on failure.
 */
export async function copyToClipboard(text: string): Promise<boolean> {
	try {
		await navigator.clipboard.writeText(text);
		return true;
	} catch {
		return false;
	}
}
