import type { ExtensionResponse } from './types';
import { listExtensions } from './api';

let extensions: ExtensionResponse[] = $state([]);
let loaded: boolean = $state(false);

export function getExtensions(): ExtensionResponse[] {
	return extensions;
}

export function getExtensionsLoaded(): boolean {
	return loaded;
}

export async function loadExtensions(): Promise<void> {
	try {
		extensions = await listExtensions();
		loaded = true;
	} catch (e) {
		console.error('Failed to load extensions:', e);
		extensions = [];
		loaded = true;
	}
}

export function clearExtensions(): void {
	extensions = [];
	loaded = false;
}

export function getPageExtensions(): ExtensionResponse[] {
	return extensions.filter((e) => e.placement.type === 'page');
}

export function getPanelExtensions(targetPage: string): ExtensionResponse[] {
	return extensions.filter((e) => e.placement.type === 'panel' && e.placement.target_page === targetPage);
}

export function getContextMenuExtensions(targetEntity: string): ExtensionResponse[] {
	return extensions.filter(
		(e) => e.placement.type === 'context_menu_group' && e.placement.target_entity === targetEntity
	);
}

export function getTableExtensions(targetTable: string): ExtensionResponse[] {
	return extensions.filter((e) => e.placement.type === 'table_columns' && e.placement.target_table === targetTable);
}
