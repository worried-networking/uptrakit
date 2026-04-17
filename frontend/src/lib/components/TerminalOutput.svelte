<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
	import { Terminal } from '@xterm/xterm';
	import { FitAddon } from '@xterm/addon-fit';
	import { WebLinksAddon } from '@xterm/addon-web-links';
	import { Callout, SectionCard, StatusBadge } from '$lib/components/ui';
	import '@xterm/xterm/css/xterm.css';

	interface Props {
		/** Static output to render (used for completed updates). */
		output?: string;
		/** Additional CSS classes for the container. */
		class?: string;
		/**
		 * When provided, stdin is enabled and each keypress / paste in the
		 * terminal calls this handler with the raw xterm data string.
		 */
		onInput?: (data: string) => void;
	}

	let { output, class: className = '', onInput }: Props = $props();

	let containerEl: HTMLDivElement | undefined = $state(undefined);
	let terminal: Terminal | null = $state(null);
	let fitAddon: FitAddon | null = null;
	let resizeObserver: ResizeObserver | null = null;
	let themeObserver: MutationObserver | null = null;
	let viewportWidth = $state(1024);
	let liveMode = $derived(onInput !== undefined);
	const MOBILE_BREAKPOINT = 640;

	const DARK_THEME = {
		background: '#1e1e2e',
		foreground: '#cdd6f4',
		cursor: '#f5e0dc',
		selectionBackground: '#585b70',
		black: '#45475a',
		red: '#f38ba8',
		green: '#a6e3a1',
		yellow: '#f9e2af',
		blue: '#89b4fa',
		magenta: '#f5c2e7',
		cyan: '#94e2d5',
		white: '#bac2de',
		brightBlack: '#585b70',
		brightRed: '#f38ba8',
		brightGreen: '#a6e3a1',
		brightYellow: '#f9e2af',
		brightBlue: '#89b4fa',
		brightMagenta: '#f5c2e7',
		brightCyan: '#94e2d5',
		brightWhite: '#a6adc8'
	};

	const LIGHT_THEME = {
		background: '#eff1f5',
		foreground: '#4c4f69',
		cursor: '#dc8a78',
		selectionBackground: '#acb0be',
		black: '#5c5f77',
		red: '#d20f39',
		green: '#40a02b',
		yellow: '#df8e1d',
		blue: '#1e66f5',
		magenta: '#ea76cb',
		cyan: '#179299',
		white: '#acb0be',
		brightBlack: '#6c6f85',
		brightRed: '#d20f39',
		brightGreen: '#40a02b',
		brightYellow: '#df8e1d',
		brightBlue: '#1e66f5',
		brightMagenta: '#ea76cb',
		brightCyan: '#179299',
		brightWhite: '#bcc0cc'
	};

	function isDarkMode(): boolean {
		return document.documentElement.classList.contains('dark');
	}

	function getTheme() {
		return isDarkMode() ? DARK_THEME : LIGHT_THEME;
	}

	onMount(() => {
		if (!containerEl) return;

		const syncViewport = () => {
			viewportWidth = window.innerWidth;
			fitAddon?.fit();
		};

		terminal = new Terminal({
			disableStdin: onInput === undefined,
			// convertEol only for static (non-PTY) output: stored output uses plain
			// \n endings, but PTY output already carries \r\n so no conversion is
			// needed — enabling it in interactive mode causes a double-newline on
			// every echoed character.
			convertEol: onInput === undefined,
			scrollback: 10000,
			fontSize: 13,
			fontFamily: 'ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace',
			theme: getTheme(),
			cursorBlink: onInput !== undefined,
			cursorStyle: 'bar',
			cursorInactiveStyle: 'none'
		});

		fitAddon = new FitAddon();
		terminal.loadAddon(fitAddon);
		terminal.loadAddon(new WebLinksAddon());

		terminal.open(containerEl);
		fitAddon.fit();

		if (onInput) {
			terminal.onData(onInput);
		}

		// Auto-resize when the container changes size.
		resizeObserver = new ResizeObserver(() => {
			fitAddon?.fit();
		});
		resizeObserver.observe(containerEl);

		// Sync theme with dark/light mode changes.
		themeObserver = new MutationObserver(() => {
			if (terminal) {
				terminal.options.theme = getTheme();
			}
		});
		themeObserver.observe(document.documentElement, {
			attributes: true,
			attributeFilter: ['class']
		});

		syncViewport();
		window.addEventListener('resize', syncViewport);

		return () => {
			window.removeEventListener('resize', syncViewport);
		};
	});

	onDestroy(() => {
		resizeObserver?.disconnect();
		themeObserver?.disconnect();
		terminal?.dispose();
	});

	// When `output` prop changes, rewrite the terminal.
	$effect(() => {
		if (terminal && output !== undefined) {
			terminal.clear();
			terminal.write(output);
		}
	});

	$effect(() => {
		if (!liveMode || !containerEl || viewportWidth >= MOBILE_BREAKPOINT) return;

		const modalShell = containerEl.closest<HTMLElement>('[data-ui="modal-shell"]');
		const modalFrame = modalShell?.parentElement;
		const modalContent = modalShell?.firstElementChild as HTMLElement | null;
		const modalFooter = modalShell?.lastElementChild as HTMLElement | null;

		if (!modalShell || !modalFrame) return;

		const previousShellStyles = {
			width: modalShell.style.width,
			height: modalShell.style.height,
			maxWidth: modalShell.style.maxWidth,
			maxHeight: modalShell.style.maxHeight,
			borderRadius: modalShell.style.borderRadius,
			borderLeft: modalShell.style.borderLeft,
			borderRight: modalShell.style.borderRight,
			borderBottom: modalShell.style.borderBottom
		};
		const previousFrameStyles = {
			padding: modalFrame.style.padding,
			alignItems: modalFrame.style.alignItems
		};
		const previousContentStyles = modalContent
			? {
					display: modalContent.style.display,
					flex: modalContent.style.flex,
					flexDirection: modalContent.style.flexDirection,
					minHeight: modalContent.style.minHeight,
					paddingLeft: modalContent.style.paddingLeft,
					paddingRight: modalContent.style.paddingRight,
					paddingTop: modalContent.style.paddingTop,
					paddingBottom: modalContent.style.paddingBottom
				}
			: null;
		const previousFooterStyles = modalFooter
			? {
					paddingLeft: modalFooter.style.paddingLeft,
					paddingRight: modalFooter.style.paddingRight,
					paddingBottom: modalFooter.style.paddingBottom
				}
			: null;

		modalFrame.style.padding = '0';
		modalFrame.style.alignItems = 'stretch';
		modalShell.style.width = '100vw';
		modalShell.style.height = '100dvh';
		modalShell.style.maxWidth = 'none';
		modalShell.style.maxHeight = '100dvh';
		modalShell.style.borderRadius = '0';
		modalShell.style.borderLeft = '0';
		modalShell.style.borderRight = '0';
		modalShell.style.borderBottom = '0';

		if (modalContent) {
			modalContent.style.display = 'flex';
			modalContent.style.flex = '1';
			modalContent.style.flexDirection = 'column';
			modalContent.style.minHeight = '0';
			modalContent.style.paddingLeft = '1rem';
			modalContent.style.paddingRight = '1rem';
			modalContent.style.paddingTop = '1rem';
			modalContent.style.paddingBottom = '0.75rem';
		}

		if (modalFooter) {
			modalFooter.style.paddingLeft = '1rem';
			modalFooter.style.paddingRight = '1rem';
			modalFooter.style.paddingBottom = 'calc(1rem + env(safe-area-inset-bottom))';
		}

		return () => {
			modalFrame.style.padding = previousFrameStyles.padding;
			modalFrame.style.alignItems = previousFrameStyles.alignItems;
			modalShell.style.width = previousShellStyles.width;
			modalShell.style.height = previousShellStyles.height;
			modalShell.style.maxWidth = previousShellStyles.maxWidth;
			modalShell.style.maxHeight = previousShellStyles.maxHeight;
			modalShell.style.borderRadius = previousShellStyles.borderRadius;
			modalShell.style.borderLeft = previousShellStyles.borderLeft;
			modalShell.style.borderRight = previousShellStyles.borderRight;
			modalShell.style.borderBottom = previousShellStyles.borderBottom;

			if (modalContent && previousContentStyles) {
				modalContent.style.display = previousContentStyles.display;
				modalContent.style.flex = previousContentStyles.flex;
				modalContent.style.flexDirection = previousContentStyles.flexDirection;
				modalContent.style.minHeight = previousContentStyles.minHeight;
				modalContent.style.paddingLeft = previousContentStyles.paddingLeft;
				modalContent.style.paddingRight = previousContentStyles.paddingRight;
				modalContent.style.paddingTop = previousContentStyles.paddingTop;
				modalContent.style.paddingBottom = previousContentStyles.paddingBottom;
			}

			if (modalFooter && previousFooterStyles) {
				modalFooter.style.paddingLeft = previousFooterStyles.paddingLeft;
				modalFooter.style.paddingRight = previousFooterStyles.paddingRight;
				modalFooter.style.paddingBottom = previousFooterStyles.paddingBottom;
			}
		};
	});

	/** Write data to the terminal (for streaming mode). */
	export function write(data: string) {
		// Interactive mode uses convertEol:false because PTY output already carries
		// \r\n. Synthesized system/error messages from the agent use plain \n, which
		// causes incorrect indentation in raw PTY mode. Normalize bare \n to \r\n
		// without touching \r\n pairs (lookbehind ensures no double-conversion).
		terminal?.write(data.replace(/(?<!\r)\n/g, '\r\n'));
	}

	/** Clear the terminal. */
	export function clear() {
		terminal?.clear();
	}
</script>

<div
	class={`terminal-output-shell ${className}`}
	data-ui="terminal-output-shell"
	data-live={liveMode ? 'true' : 'false'}
>
	<SectionCard title="Terminal output">
		{#snippet actions()}
			<StatusBadge tone={liveMode ? 'info' : 'neutral'} label={liveMode ? 'Live' : 'Captured'} />
		{/snippet}

		{#if liveMode}
			<div class="mb-3">
				<Callout
					tone="info"
					title="Interactive input enabled"
					message="Typed input is forwarded directly to the active remote session."
				/>
			</div>
		{/if}

		<div bind:this={containerEl} class="terminal-output" data-ui="terminal-output"></div>
	</SectionCard>
</div>

<style>
	.terminal-output-shell {
		min-height: 0;
	}

	.terminal-output-shell :global([data-ui='section-card']) {
		display: flex;
		height: 100%;
		min-height: 0;
		flex-direction: column;
	}

	.terminal-output-shell :global([data-ui='section-card'] > div:last-child) {
		display: flex;
		min-height: 0;
		flex: 1;
		flex-direction: column;
	}

	.terminal-output {
		min-height: 200px;
		height: 100%;
		flex: 1;
		border-radius: 0.5rem;
		overflow: hidden;
		border: 1px solid var(--border-subtle);
		background: var(--bg-base);
	}

	.terminal-output :global(.xterm) {
		padding: 0.5rem;
	}

	@media (max-width: 639px) {
		.terminal-output-shell[data-live='true'] {
			display: flex;
			height: calc(100dvh - 9.5rem);
			flex-direction: column;
		}
	}
</style>
