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
