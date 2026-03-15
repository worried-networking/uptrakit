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
 * Validates that a URL is safe to render as an external link.
 * Allows only http(s) URLs.
 */
export function isValidExternalUrl(url: string | null | undefined): boolean {
	if (!url) return false;
	try {
		const parsed = new URL(url);
		return parsed.protocol === 'https:' || parsed.protocol === 'http:';
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
	if (/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/.test(version)) {
		return new Date(version).toLocaleString(undefined, {
			year: 'numeric',
			month: 'short',
			day: 'numeric',
			hour: '2-digit',
			minute: '2-digit'
		});
	}
	return version;
}

/**
 * Returns the most informative version string for display.
 * Prefers `displayVersion` when set by the plugin (e.g. Docker publish date).
 * Falls back to `canonicalVersion` for plugins that use human-readable versions.
 */
export function resolveDisplayVersion(
	canonicalVersion: string | null | undefined,
	displayVersion: string | null | undefined
): string | null | undefined {
	return displayVersion ?? canonicalVersion;
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

const API_SUBMIT_PLACEHOLDER = /^\{\{(\w+)(?::(\w+))?\}\}$/;
const MAX_CSV_ARRAY_ITEMS = 100;

/**
 * Renders an `api_submit` template by replacing `{{field}}` leaves with values.
 * Throws on invalid coercions so callers can surface actionable errors.
 */
export function renderApiSubmitTemplate(template: unknown, values: Record<string, unknown>): unknown {
	if (typeof template === 'string') {
		const match = template.match(API_SUBMIT_PLACEHOLDER);
		if (!match) return template;
		const [, fieldName, coercion] = match;
		if (!(fieldName in values)) {
			throw new Error(`Unknown form field "${fieldName}" in action template`);
		}
		const rawValue = values[fieldName];
		const raw = typeof rawValue === 'string' ? rawValue : String(rawValue ?? '');

		if (!coercion) return raw;
		if (coercion === 'bool') return raw === 'true';
		if (coercion === 'number') {
			const number = Number(raw);
			if (!Number.isFinite(number)) {
				throw new Error(`Field "${fieldName}" must be a valid number`);
			}
			return number;
		}
		if (coercion === 'csv_array') {
			const items = raw
				.split(',')
				.map((item) => item.trim())
				.filter(Boolean);
			if (items.length > MAX_CSV_ARRAY_ITEMS) {
				throw new Error(`Field "${fieldName}" exceeds the ${MAX_CSV_ARRAY_ITEMS}-item limit`);
			}
			return items;
		}
		throw new Error(`Unsupported action template coercion "${coercion}"`);
	}

	if (Array.isArray(template)) {
		return template.map((item) => renderApiSubmitTemplate(item, values));
	}

	if (template !== null && typeof template === 'object') {
		return Object.fromEntries(
			Object.entries(template as Record<string, unknown>).map(([key, value]) => [
				key,
				renderApiSubmitTemplate(value, values)
			])
		);
	}

	return template;
}
