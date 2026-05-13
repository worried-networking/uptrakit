/**
 * HTML-escape an attacker-controlled string. The OAuth consent screen renders
 * client_name and client_uri values that are supplied by DCR registrants or
 * fetched from CIMD documents — both are attacker-controlled. Every binding
 * site must funnel these strings through `htmlEscape()` to prevent HTML
 * injection in the consent prompt.
 */
export function htmlEscape(input: string): string {
	return input
		.replace(/&/g, '&amp;')
		.replace(/</g, '&lt;')
		.replace(/>/g, '&gt;')
		.replace(/"/g, '&quot;')
		.replace(/'/g, '&#39;');
}
