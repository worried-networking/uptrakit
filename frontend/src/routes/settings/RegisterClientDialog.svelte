<script lang="ts">
	import { manualRegisterClient } from '$lib/api';
	import { ModalShell, Callout } from '$lib/components/ui';
	import { FormFieldRow, Input, Select, Textarea } from '$lib/components/forms';
	import Button from '$lib/components/Button.svelte';

	let {
		open,
		onClose,
		onSuccess
	}: {
		open: boolean;
		onClose: () => void;
		onSuccess: () => void;
	} = $props();

	let clientName: string = $state('');
	let clientUri: string = $state('');
	let redirectUrisRaw: string = $state('');
	let defaultScope: string = $state('mcp:read');
	let tokenEndpointAuthMethod = $state<'none' | 'client_secret_basic'>('none');
	let submitting: boolean = $state(false);
	let submitError: string | null = $state(null);
	let fieldErrors: Record<string, string> = $state({});

	const scopeOptions = [
		{ value: 'mcp:read', label: 'mcp:read' },
		{ value: 'mcp:write', label: 'mcp:write' }
	];

	function parseRedirectUris(raw: string): string[] {
		return raw
			.split('\n')
			.map((u) => u.trim())
			.filter((u) => u.length > 0);
	}

	function isValidRedirectUri(uri: string): boolean {
		try {
			const parsed = new URL(uri);
			if (parsed.protocol === 'https:') return true;
			if (parsed.protocol === 'http:') {
				const hostname = parsed.hostname;
				return hostname === 'localhost' || hostname === '127.0.0.1' || hostname === '::1';
			}
			return false;
		} catch {
			return false;
		}
	}

	function validate(): boolean {
		const errors: Record<string, string> = {};

		if (!clientName.trim()) {
			errors['client_name'] = 'Client name is required.';
		}

		if (clientUri.trim()) {
			try {
				const parsed = new URL(clientUri.trim());
				if (parsed.protocol !== 'https:' && parsed.protocol !== 'http:') {
					errors['client_uri'] = 'Client URI must be an HTTP or HTTPS URL.';
				}
			} catch {
				errors['client_uri'] = 'Client URI must be a valid URL.';
			}
		}

		const uris = parseRedirectUris(redirectUrisRaw);
		if (uris.length === 0) {
			errors['redirect_uris'] = 'At least one redirect URI is required.';
		} else {
			const invalid = uris.filter((u) => !isValidRedirectUri(u));
			if (invalid.length > 0) {
				errors['redirect_uris'] =
					`Invalid redirect URI(s): ${invalid.join(', ')}. Only HTTPS or localhost URLs are allowed.`;
			}
		}

		fieldErrors = errors;
		return Object.keys(errors).length === 0;
	}

	async function handleSubmit() {
		if (!validate()) return;

		submitting = true;
		submitError = null;
		try {
			await manualRegisterClient({
				body: {
					client_name: clientName.trim(),
					client_uri: clientUri.trim() || undefined,
					redirect_uris: parseRedirectUris(redirectUrisRaw),
					grant_types: ['authorization_code'],
					response_types: ['code'],
					token_endpoint_auth_method: tokenEndpointAuthMethod,
					scope: defaultScope
				}
			});
			resetForm();
			onSuccess();
			onClose();
		} catch (e) {
			submitError = e instanceof Error ? e.message : 'Failed to register client';
		} finally {
			submitting = false;
		}
	}

	function resetForm() {
		clientName = '';
		clientUri = '';
		redirectUrisRaw = '';
		defaultScope = 'mcp:read';
		tokenEndpointAuthMethod = 'none';
		fieldErrors = {};
		submitError = null;
	}

	function handleClose() {
		resetForm();
		onClose();
	}
</script>

{#if open}
	<ModalShell title="Register OAuth Client" onclose={handleClose} maxWidth="max-w-lg">
		<div class="space-y-4">
			{#if submitError}
				<Callout tone="danger" message={submitError} />
			{/if}

			<FormFieldRow label="Client name" inputId="reg-client-name" required error={fieldErrors['client_name']}>
				<Input
					id="reg-client-name"
					type="text"
					bind:value={clientName}
					placeholder="My MCP Client"
					error={fieldErrors['client_name']}
					oninput={() => {
						fieldErrors = { ...fieldErrors, client_name: '' };
					}}
				/>
			</FormFieldRow>

			<FormFieldRow
				label="Client URI"
				inputId="reg-client-uri"
				hint="Optional. Homepage or documentation URL for this client."
				error={fieldErrors['client_uri']}
			>
				<Input
					id="reg-client-uri"
					type="url"
					bind:value={clientUri}
					placeholder="https://example.com"
					error={fieldErrors['client_uri']}
					oninput={() => {
						fieldErrors = { ...fieldErrors, client_uri: '' };
					}}
				/>
			</FormFieldRow>

			<FormFieldRow
				label="Redirect URIs"
				inputId="reg-redirect-uris"
				required
				hint="One URL per line. Must be HTTPS or localhost."
				error={fieldErrors['redirect_uris']}
			>
				<Textarea
					id="reg-redirect-uris"
					bind:value={redirectUrisRaw}
					placeholder="https://example.com/callback"
					rows={4}
					error={fieldErrors['redirect_uris']}
					oninput={() => {
						fieldErrors = { ...fieldErrors, redirect_uris: '' };
					}}
				/>
			</FormFieldRow>

			<FormFieldRow label="Default scope" inputId="reg-default-scope">
				<Select id="reg-default-scope" bind:value={defaultScope} options={scopeOptions} />
			</FormFieldRow>

			<div class="space-y-1">
				<span class="block text-xs text-[var(--text-muted)]">Auth method</span>
				<label class="flex items-center gap-2 text-sm text-[var(--text-primary)]">
					<input type="radio" bind:group={tokenEndpointAuthMethod} value="none" />
					Public client (none)
				</label>
				<label class="flex items-center gap-2 text-sm text-[var(--text-primary)]">
					<input type="radio" bind:group={tokenEndpointAuthMethod} value="client_secret_basic" />
					Confidential client (client_secret_basic)
				</label>
			</div>
		</div>

		{#snippet footer()}
			<Button variant="secondary" disabled={submitting} onclick={handleClose}>Cancel</Button>
			<Button variant="primary" loading={submitting} onclick={() => void handleSubmit()}>Register</Button>
		{/snippet}
	</ModalShell>
{/if}
