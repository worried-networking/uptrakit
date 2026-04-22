import type { Plugin } from 'vite';
import { cssForTheme } from '../src/theme/tokens';

export const VIRTUAL_ID = 'virtual:theme/tokens.css';
const RESOLVED_VIRTUAL_ID = '\0' + VIRTUAL_ID;
const TOKENS_SOURCE_SUFFIX = 'src/theme/tokens.ts';

export function themeTokensPlugin(): Plugin {
	return {
		name: 'uptrakit:theme-tokens',
		resolveId(id) {
			if (id === VIRTUAL_ID) return RESOLVED_VIRTUAL_ID;
			return undefined;
		},
		load(id) {
			if (id !== RESOLVED_VIRTUAL_ID) return undefined;
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
