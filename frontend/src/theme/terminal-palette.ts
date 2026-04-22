import type { ITheme } from '@xterm/xterm';
import { tokens } from './tokens';

const SUCCESS = tokens['--color-success'].dark;
const ACCENT_BRIGHT = tokens['--accent-bright'].dark;
const MUTED = tokens['--text-muted'].dark;
const PRIMARY = tokens['--text-primary'].dark;
const INVERTED = tokens['--text-inverted'].dark;
const INFO = tokens['--color-info'].dark;

// ANSI-only colors — not part of design language, kept local.
const TERM_BG = '#0c0c0e';
const TERM_FG = '#d4d4d8';
const SELECTION = '#3f3f46';
const ANSI_BLACK = '#18181b';
const ANSI_RED = '#f87171';
const ANSI_BLUE = '#60a5fa';
const ANSI_MAGENTA = '#c084fc';
const ANSI_BRIGHT_RED = '#fb7185';
const ANSI_BRIGHT_GREEN = '#86efac';
const ANSI_BRIGHT_YELLOW = '#fde68a';
const ANSI_BRIGHT_BLUE = '#93c5fd';
const ANSI_BRIGHT_MAGENTA = '#d8b4fe';

// Parent spec §6 pins terminal yellow at `#fcd34d` (progress / in-flight
// layers). This is distinct from `--color-warning` dark (`#fbbf24`) by
// design — terminal amber sits higher on the ramp for readability.
const TERMINAL_AMBER = '#fcd34d';

export const TERMINAL_THEME: ITheme = {
	background: TERM_BG,
	foreground: TERM_FG,
	cursor: TERM_FG,
	selectionBackground: SELECTION,
	black: ANSI_BLACK,
	red: ANSI_RED,
	green: SUCCESS,
	yellow: TERMINAL_AMBER,
	blue: ANSI_BLUE,
	magenta: ANSI_MAGENTA,
	cyan: ACCENT_BRIGHT,
	white: PRIMARY,
	brightBlack: MUTED,
	brightRed: ANSI_BRIGHT_RED,
	brightGreen: ANSI_BRIGHT_GREEN,
	brightYellow: ANSI_BRIGHT_YELLOW,
	brightBlue: ANSI_BRIGHT_BLUE,
	brightMagenta: ANSI_BRIGHT_MAGENTA,
	brightCyan: INFO,
	brightWhite: INVERTED
};
