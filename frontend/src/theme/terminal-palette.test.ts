import { describe, expect, it } from 'vitest';
import { TERMINAL_THEME } from './terminal-palette';
import { tokens } from './tokens';

describe('TERMINAL_THEME bindings to tokens.ts (parent spec §6)', () => {
	it('brightBlack — timestamps / layer IDs — uses --text-muted dark', () => {
		expect(TERMINAL_THEME.brightBlack).toBe(tokens['--text-muted'].dark);
	});

	it('cyan — uptrakit annotations — uses --accent-bright dark', () => {
		expect(TERMINAL_THEME.cyan).toBe(tokens['--accent-bright'].dark);
	});

	it('brightWhite — Docker status lines — uses --text-inverted dark', () => {
		expect(TERMINAL_THEME.brightWhite).toBe(tokens['--text-inverted'].dark);
	});

	it('green — success lines — uses --color-success dark', () => {
		expect(TERMINAL_THEME.green).toBe(tokens['--color-success'].dark);
	});

	it('white — default text — uses --text-primary dark', () => {
		expect(TERMINAL_THEME.white).toBe(tokens['--text-primary'].dark);
	});

	it('brightCyan — bright info — uses --color-info dark', () => {
		expect(TERMINAL_THEME.brightCyan).toBe(tokens['--color-info'].dark);
	});

	it('yellow — terminal amber — pins #fcd34d per §6 (distinct from --color-warning)', () => {
		expect(TERMINAL_THEME.yellow).toBe('#fcd34d');
		expect(TERMINAL_THEME.yellow).not.toBe(tokens['--color-warning'].dark);
	});

	it('background — #0c0c0e — always-dark body per §6', () => {
		expect(TERMINAL_THEME.background).toBe('#0c0c0e');
	});

	it('snapshot: full TERMINAL_THEME object', () => {
		expect(TERMINAL_THEME).toMatchInlineSnapshot(`
{
  "background": "#0c0c0e",
  "black": "#18181b",
  "blue": "#60a5fa",
  "brightBlack": "#52525b",
  "brightBlue": "#93c5fd",
  "brightCyan": "#67e8f9",
  "brightGreen": "#86efac",
  "brightMagenta": "#d8b4fe",
  "brightRed": "#fb7185",
  "brightWhite": "#fafafa",
  "brightYellow": "#fde68a",
  "cursor": "#d4d4d8",
  "cyan": "#22d3ee",
  "foreground": "#d4d4d8",
  "green": "#4ade80",
  "magenta": "#c084fc",
  "red": "#f87171",
  "selectionBackground": "#3f3f46",
  "white": "#e4e4e7",
  "yellow": "#fcd34d",
}
`);
	});
});
