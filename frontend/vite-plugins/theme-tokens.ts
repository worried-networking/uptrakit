import type { Plugin } from 'vite';
import { cssForTheme } from '../src/theme/tokens';

export const VIRTUAL_ID = 'virtual:theme/tokens.css';
const RESOLVED_VIRTUAL_ID = '\0' + VIRTUAL_ID;
const TOKENS_SOURCE_SUFFIX = 'src/theme/tokens.ts';
const VIRTUAL_IMPORT_RE = /@import\s+['"]virtual:theme\/tokens\.css['"]\s*;?/g;

function buildTokensCss(): string {
	return [
		':root {',
		'  color-scheme: light;',
		cssForTheme('light'),
		'}',
		'.dark {',
		'  color-scheme: dark;',
		cssForTheme('dark'),
		'}',
		''
	].join('\n');
}

export function themeTokensPlugin(): Plugin {
	return {
		name: 'uptrakit:theme-tokens',
		// Inline the virtual import in any CSS file before Tailwind's CSS resolver runs.
		// @tailwindcss/vite uses its own CSS resolver that does not traverse Vite plugin
		// resolveId hooks, so we expand the @import here during the transform phase instead.
		transform: {
			order: 'pre',
			handler(code, id) {
				if (!id.endsWith('.css') && !id.includes('.css?')) return undefined;
				if (!VIRTUAL_IMPORT_RE.test(code)) return undefined;
				VIRTUAL_IMPORT_RE.lastIndex = 0;
				return { code: code.replace(VIRTUAL_IMPORT_RE, buildTokensCss()), map: null };
			}
		},
		resolveId(id) {
			if (id === VIRTUAL_ID) return RESOLVED_VIRTUAL_ID;
			return undefined;
		},
		load(id) {
			if (id !== RESOLVED_VIRTUAL_ID) return undefined;
			return buildTokensCss();
		},
		handleHotUpdate({ file, server }) {
			if (!file.endsWith(TOKENS_SOURCE_SUFFIX)) return;
			const virtualModule = server.moduleGraph.getModuleById(RESOLVED_VIRTUAL_ID);
			if (!virtualModule) return;
			server.moduleGraph.invalidateModule(virtualModule);
			return [virtualModule];
		}
	};
}
