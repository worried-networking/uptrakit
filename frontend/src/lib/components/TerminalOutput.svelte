<script lang="ts">
	import { onMount, onDestroy } from 'svelte';
	import { Terminal } from '@xterm/xterm';
	import { FitAddon } from '@xterm/addon-fit';
	import { WebLinksAddon } from '@xterm/addon-web-links';
	import { Callout, StatusBadge } from '$lib/components/ui';
	import type { CalloutTone } from '$lib/components/ui/Callout.svelte';
	import type { StatusBadgeTone } from '$lib/components/ui/StatusBadge.svelte';
	import '@xterm/xterm/css/xterm.css';

	type TerminalCallout = {
		tone: CalloutTone;
		title?: string;
		message: string;
	};

	type TerminalAction = {
		id?: string;
		label: string;
		title?: string;
		tone?: 'neutral' | 'danger';
		disabled?: boolean;
		onclick: () => void;
	};

	interface Props {
		open?: boolean;
		title?: string;
		statusLabel?: string;
		statusTone?: StatusBadgeTone;
		metadata?: string;
		output?: string;
		/**
		 * When provided, stdin is enabled and each keypress / paste in the
		 * terminal calls this handler with the raw xterm data string.
		 */
		onInput?: (data: string) => void;
		onclose: () => void;
		/** If false, the modal shows callouts only without opening xterm. */
		showTerminal?: boolean;
		callouts?: TerminalCallout[];
		actions?: TerminalAction[];
		class?: string;
	}

	let {
		open = true,
		title = 'Terminal Output',
		statusLabel = 'Captured',
		statusTone = 'neutral',
		metadata = '',
		output,
		onInput,
		onclose,
		showTerminal = true,
		callouts = [],
		actions = [],
		class: className = ''
	}: Props = $props();

	let containerEl: HTMLDivElement | undefined = $state(undefined);
	let terminal: Terminal | null = null;
	let fitAddon: FitAddon | null = null;
	let resizeObserver: ResizeObserver | null = null;
	let inputSubscription: { dispose: () => void } | null = null;
	let viewportWidth = $state(1024);
	let maximized = $state(false);
	let isHoveringDots = $state(false);

	const MOBILE_BREAKPOINT = 640;
	const liveMode = $derived(typeof onInput === 'function');
	const isMobile = $derived(viewportWidth < MOBILE_BREAKPOINT);
	const maximizeVisible = $derived(!isMobile);

	const TERMINAL_THEME = {
		background: '#0c0c0e',
		foreground: '#d4d4d8',
		cursor: '#d4d4d8',
		selectionBackground: '#3f3f46',
		black: '#18181b',
		red: '#f87171',
		green: '#4ade80',
		yellow: '#fcd34d',
		blue: '#60a5fa',
		magenta: '#c084fc',
		cyan: '#22d3ee',
		white: '#e4e4e7',
		brightBlack: '#3f3f46',
		brightRed: '#fb7185',
		brightGreen: '#86efac',
		brightYellow: '#fde68a',
		brightBlue: '#93c5fd',
		brightMagenta: '#d8b4fe',
		brightCyan: '#67e8f9',
		brightWhite: '#fafafa'
	};

	function syncViewport() {
		viewportWidth = window.innerWidth;
		fitAddon?.fit();
	}

	function requestClose() {
		maximized = false;
		onclose();
	}

	function toggleMaximize() {
		if (!maximizeVisible) return;
		maximized = !maximized;
		setTimeout(() => {
			fitAddon?.fit();
		}, 0);
	}

	function handleWindowKeydown(event: KeyboardEvent) {
		if (!open) return;
		if (event.key !== 'Escape') return;
		event.preventDefault();
		requestClose();
	}

	onMount(() => {
		syncViewport();
		window.addEventListener('resize', syncViewport);
		window.addEventListener('keydown', handleWindowKeydown);
	});

	onDestroy(() => {
		window.removeEventListener('resize', syncViewport);
		window.removeEventListener('keydown', handleWindowKeydown);
		resizeObserver?.disconnect();
		terminal?.dispose();
	});

	$effect(() => {
		if (!open || !showTerminal || !containerEl || terminal) return;

		const nextTerminal = new Terminal({
			disableStdin: !liveMode,
			convertEol: !liveMode,
			scrollback: 10000,
			fontSize: 9,
			lineHeight: 1.6,
			fontFamily: 'ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace',
			theme: TERMINAL_THEME,
			cursorBlink: liveMode,
			cursorStyle: 'bar',
			cursorInactiveStyle: 'none'
		});

		fitAddon = new FitAddon();
		nextTerminal.loadAddon(fitAddon);
		nextTerminal.loadAddon(new WebLinksAddon());
		nextTerminal.open(containerEl);
		fitAddon.fit();

		resizeObserver = new ResizeObserver(() => {
			fitAddon?.fit();
		});
		resizeObserver.observe(containerEl);

		terminal = nextTerminal;

		return () => {
			resizeObserver?.disconnect();
			resizeObserver = null;
			nextTerminal.dispose();
			terminal = null;
			fitAddon = null;
		};
	});

	$effect(() => {
		if (!terminal) return;
		inputSubscription?.dispose();
		inputSubscription = null;
		terminal.options.disableStdin = !liveMode;
		terminal.options.cursorBlink = liveMode;
		terminal.options.convertEol = !liveMode;
		if (typeof onInput === 'function') {
			inputSubscription = terminal.onData(onInput);
		}
		return () => {
			inputSubscription?.dispose();
			inputSubscription = null;
		};
	});

	$effect(() => {
		if (!terminal || liveMode) return;
		terminal.clear();
		terminal.write(output ?? '');
	});

	/** Write data to the terminal (for streaming mode). */
	export function write(data: string) {
		// Interactive mode uses convertEol:false because PTY output already carries
		// \r\n. Synthesized system/error messages can still use plain \n.
		terminal?.write(data.replace(/(?<!\r)\n/g, '\r\n'));
	}

	/** Clear the terminal. */
	export function clear() {
		terminal?.clear();
	}
</script>

{#if open}
	<div
		class="terminal-backdrop"
		data-ui="terminal-backdrop"
		role="presentation"
		onclick={(event) => {
			if (event.target === event.currentTarget) requestClose();
		}}
	>
		<div
			class={`terminal-shell ${className}`}
			data-ui="terminal-shell"
			data-maximized={maximized ? 'true' : 'false'}
			role="dialog"
			aria-modal="true"
			aria-label={title}
		>
			<header class="terminal-titlebar" data-ui="terminal-titlebar">
				<div
					class="terminal-dots"
					role="group"
					aria-label="Terminal controls"
					data-hovering={isHoveringDots ? 'true' : 'false'}
					onmouseenter={() => {
						isHoveringDots = true;
					}}
					onmouseleave={() => {
						isHoveringDots = false;
					}}
				>
					<button
						type="button"
						class="terminal-dot terminal-dot--red"
						aria-label="Close terminal"
						onclick={requestClose}
					>
						<span class="terminal-dot-icon" aria-hidden="true">✕</span>
					</button>
					<button
						type="button"
						class="terminal-dot terminal-dot--yellow"
						aria-label="Minimize terminal"
						disabled
						tabindex="-1"
					>
						<span class="terminal-dot-icon" aria-hidden="true">_</span>
					</button>
					{#if maximizeVisible}
						<button
							type="button"
							class="terminal-dot terminal-dot--green"
							aria-label={maximized ? 'Restore terminal' : 'Maximize terminal'}
							onclick={toggleMaximize}
						>
							<span class="terminal-dot-icon" aria-hidden="true">{maximized ? '⊡' : '+'}</span>
						</button>
					{/if}
				</div>
				<p class="terminal-title" data-ui="terminal-title">{title}</p>
				<div class="terminal-titlebar-spacer" aria-hidden="true"></div>
			</header>

			<div class="terminal-body" data-ui="terminal-body">
				{#if callouts.length > 0}
					<div class="terminal-callouts">
						{#each callouts as callout (`${callout.tone}-${callout.title ?? ''}-${callout.message}`)}
							<Callout tone={callout.tone} title={callout.title} message={callout.message} />
						{/each}
					</div>
				{/if}
				{#if showTerminal}
					<div bind:this={containerEl} class="terminal-output" data-ui="terminal-output"></div>
				{/if}
			</div>

			<footer class="terminal-statusbar" data-ui="terminal-statusbar">
				<div class="terminal-status-leading">
					<StatusBadge tone={statusTone} label={statusLabel} />
				</div>
				<div class="terminal-status-trailing">
					{#if actions.length > 0}
						<div class="terminal-actions" data-ui="terminal-actions">
							{#each actions as action (action.id ?? action.label)}
								<button
									type="button"
									class={`terminal-action terminal-action--${action.tone ?? 'neutral'}`}
									title={action.title}
									disabled={action.disabled}
									onclick={action.onclick}
								>
									{action.label}
								</button>
							{/each}
						</div>
					{/if}
					<span class="terminal-metadata">{metadata}</span>
				</div>
			</footer>
		</div>
	</div>
{/if}

<style>
	.terminal-backdrop {
		position: fixed;
		inset: 0;
		z-index: 900;
		display: flex;
		align-items: center;
		justify-content: center;
		padding: 1rem;
		background: rgba(0, 0, 0, 0.78);
	}

	.terminal-shell {
		position: relative;
		z-index: 910;
		display: flex;
		width: 580px;
		height: 380px;
		max-width: 92vw;
		max-height: 88vh;
		flex-direction: column;
		overflow: hidden;
		border: 1px solid #27272a;
		border-radius: 6px;
		background: #0c0c0e;
		box-shadow: 0 22px 60px rgba(0, 0, 0, 0.55);
		transition:
			width 0.18s ease,
			height 0.18s ease,
			border-radius 0.18s ease;
	}

	.terminal-shell[data-maximized='true'] {
		width: 92vw;
		height: 88vh;
		border-radius: 4px;
	}

	.terminal-titlebar {
		display: grid;
		height: 36px;
		flex-shrink: 0;
		grid-template-columns: auto 1fr auto;
		align-items: center;
		border-bottom: 1px solid #27272a;
		padding-inline: 0.75rem;
		background: #111216;
	}

	.terminal-dots {
		display: inline-flex;
		gap: 0.4rem;
	}

	.terminal-dot {
		display: inline-flex;
		height: 12px;
		width: 12px;
		align-items: center;
		justify-content: center;
		border: 0;
		border-radius: 9999px;
		padding: 0;
		font-size: 8px;
		font-weight: 600;
		line-height: 1;
	}

	.terminal-dot--red {
		background: #ff5f57;
	}

	.terminal-dot--yellow {
		background: #3f3f46;
		pointer-events: none;
	}

	.terminal-dot--green {
		background: #27c840;
	}

	.terminal-dot-icon {
		color: rgba(0, 0, 0, 0.62);
		opacity: 0;
		transition: opacity 0.12s ease;
	}

	.terminal-dots:hover .terminal-dot-icon,
	.terminal-dots[data-hovering='true'] .terminal-dot-icon {
		opacity: 1;
	}

	.terminal-title {
		margin: 0;
		justify-self: center;
		font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
		font-size: 12px;
		font-weight: 500;
		color: #e4e4e7;
	}

	.terminal-titlebar-spacer {
		width: 48px;
	}

	.terminal-body {
		display: flex;
		min-height: 0;
		flex: 1;
		flex-direction: column;
		background: #0c0c0e;
	}

	.terminal-callouts {
		display: grid;
		gap: 0.5rem;
		padding: 0.6rem 0.6rem 0;
	}

	.terminal-output {
		min-height: 0;
		flex: 1;
	}

	.terminal-output :global(.xterm) {
		height: 100%;
		padding: 0.45rem 0.55rem;
	}

	.terminal-statusbar {
		display: flex;
		height: 28px;
		flex-shrink: 0;
		align-items: center;
		justify-content: space-between;
		gap: 0.5rem;
		border-top: 1px solid #27272a;
		padding: 0 0.6rem;
		background: #111216;
	}

	.terminal-status-leading {
		display: inline-flex;
		min-width: 0;
		align-items: center;
	}

	.terminal-status-trailing {
		display: inline-flex;
		min-width: 0;
		flex: 1;
		align-items: center;
		justify-content: flex-end;
		gap: 0.5rem;
	}

	.terminal-actions {
		display: inline-flex;
		gap: 0.35rem;
	}

	.terminal-action {
		height: 18px;
		border: 1px solid #3f3f46;
		border-radius: 3px;
		padding: 0 0.4rem;
		background: #18181b;
		color: #d4d4d8;
		font-size: 10px;
		font-weight: 600;
		line-height: 1;
		transition:
			background 0.12s ease,
			border-color 0.12s ease,
			color 0.12s ease;
	}

	.terminal-action:hover {
		background: #27272a;
		border-color: #52525b;
	}

	.terminal-action:disabled {
		opacity: 0.55;
		cursor: default;
	}

	.terminal-action--danger {
		border-color: #7f1d1d;
		background: #2a1113;
		color: #fda4af;
	}

	.terminal-action--danger:hover {
		border-color: #991b1b;
		background: #3a1518;
		color: #fecdd3;
	}

	.terminal-metadata {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		max-width: min(56vw, 340px);
		font-size: 11px;
		color: #a1a1aa;
	}

	@media (max-width: 1023px) {
		.terminal-shell {
			width: min(580px, 92vw);
			height: min(380px, 70vh);
		}
	}

	@media (max-width: 639px) {
		.terminal-backdrop {
			padding: 0;
		}

		.terminal-shell,
		.terminal-shell[data-maximized='true'] {
			width: 100vw;
			height: 100dvh;
			max-width: none;
			max-height: none;
			border-radius: 0;
		}

		.terminal-titlebar {
			padding-inline: 0.85rem;
		}

		.terminal-titlebar-spacer {
			width: 12px;
		}
	}
</style>
