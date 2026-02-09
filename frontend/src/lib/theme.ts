import { writable } from 'svelte/store';

export type ThemeMode = 'light' | 'dark' | 'system';

const STORAGE_KEY = 'theme-mode';

function getStored(): ThemeMode {
	if (typeof localStorage === 'undefined') return 'system';
	const v = localStorage.getItem(STORAGE_KEY);
	if (v === 'light' || v === 'dark' || v === 'system') return v;
	return 'system';
}

export const themeMode = writable<ThemeMode>(getStored());

export function applyTheme(mode: ThemeMode) {
	const prefersDark = window.matchMedia('(prefers-color-scheme: dark)').matches;
	const dark = mode === 'dark' || (mode === 'system' && prefersDark);
	document.documentElement.classList.toggle('dark', dark);
}

export function setThemeMode(mode: ThemeMode) {
	localStorage.setItem(STORAGE_KEY, mode);
	themeMode.set(mode);
	applyTheme(mode);
}

export function initTheme() {
	// The initial theme is already applied by the inline script in app.html
	// to prevent FOUC. Here we only set up the media query listener for
	// system theme changes.
	const mq = window.matchMedia('(prefers-color-scheme: dark)');
	mq.addEventListener('change', () => {
		const current = getStored();
		if (current === 'system') applyTheme('system');
	});
}
