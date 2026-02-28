<script lang="ts">
	import { createMqttClient, updateMqttClient, deleteMqttClient, getMqttLimit, updateMqttLimit } from '$lib/api';
	import {
		type CreateMqttClient,
		type MqttClientResponse,
		type MqttConnectionStatus,
		type UpdateMqttClient
	} from '$lib/types';
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import ModalBackdrop from '$lib/components/ModalBackdrop.svelte';
	import { getIsOnline } from '$lib/stores/network.svelte';

	let {
		clients,
		onSuccess,
		onError
	}: {
		clients: MqttClientResponse[] | undefined;
		onSuccess: (msg: string) => void;
		onError: (msg: string) => void;
	} = $props();

	let mqttClients: MqttClientResponse[] = $state([]);
	let showMqttModal: boolean = $state(false);
	let editingMqttClient: MqttClientResponse | null = $state(null);
	let mqttForm = $state({
		enabled: true,
		url: '',
		client_id: 'uptrakit-controller',
		username: '',
		password: '',
		ca_pem: '',
		topic_prefix: 'uptrakit',
		ha_discovery: false,
		ha_discovery_prefix: 'homeassistant'
	});
	let mqttDeleteConfirm: { id: string; url: string } | null = $state(null);
	let mqttLimit: number | null = $state(null);
	let editingLimit: boolean = $state(false);
	let limitInput: number = $state(0);
	let savingLimit: boolean = $state(false);

	$effect(() => {
		if (clients !== undefined) {
			mqttClients = clients;
			loadMqttLimit();
		}
	});

	async function loadMqttLimit() {
		try {
			const res = await getMqttLimit();
			mqttLimit = res.max_clients_per_tenant;
			limitInput = res.max_clients_per_tenant;
		} catch {
			// non-critical
		}
	}

	async function saveMqttLimit() {
		savingLimit = true;
		try {
			const res = await updateMqttLimit({ max_clients_per_tenant: limitInput });
			mqttLimit = res.max_clients_per_tenant;
			editingLimit = false;
			onSuccess('MQTT client limit updated.');
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to update MQTT limit');
		} finally {
			savingLimit = false;
		}
	}

	function openCreateMqtt() {
		editingMqttClient = null;
		mqttForm = {
			enabled: true,
			url: '',
			client_id: 'uptrakit-controller',
			username: '',
			password: '',
			ca_pem: '',
			topic_prefix: 'uptrakit',
			ha_discovery: false,
			ha_discovery_prefix: 'homeassistant'
		};
		showMqttModal = true;
	}

	function openEditMqtt(client: MqttClientResponse) {
		editingMqttClient = client;
		mqttForm = {
			enabled: client.enabled,
			url: client.url,
			client_id: client.client_id,
			username: client.username ?? '',
			password: '',
			ca_pem: '',
			topic_prefix: client.topic_prefix,
			ha_discovery: client.ha_discovery,
			ha_discovery_prefix: client.ha_discovery_prefix
		};
		showMqttModal = true;
	}

	function closeMqttModal() {
		showMqttModal = false;
		editingMqttClient = null;
	}

	async function saveMqttClient() {
		try {
			if (editingMqttClient) {
				const data: UpdateMqttClient = {
					url: mqttForm.url || undefined,
					enabled: mqttForm.enabled,
					client_id: mqttForm.client_id,
					username: mqttForm.username || null,
					topic_prefix: mqttForm.topic_prefix,
					ha_discovery: mqttForm.ha_discovery,
					ha_discovery_prefix: mqttForm.ha_discovery_prefix || undefined,
					...(mqttForm.password ? { password: mqttForm.password } : {}),
					...(mqttForm.ca_pem ? { ca_pem: mqttForm.ca_pem } : {})
				};
				const res = await updateMqttClient(editingMqttClient.id, data);
				mqttClients = mqttClients.map((c) => (c.id === res.id ? res : c));
				onSuccess('MQTT client updated.');
			} else {
				if (!mqttForm.url) {
					onError('URL is required to create an MQTT client');
					return;
				}
				const data: CreateMqttClient = {
					url: mqttForm.url,
					enabled: mqttForm.enabled,
					client_id: mqttForm.client_id || undefined,
					username: mqttForm.username || undefined,
					password: mqttForm.password || undefined,
					ca_pem: mqttForm.ca_pem || undefined,
					topic_prefix: mqttForm.topic_prefix || undefined,
					ha_discovery: mqttForm.ha_discovery,
					ha_discovery_prefix: mqttForm.ha_discovery_prefix || undefined
				};
				const res = await createMqttClient(data);
				mqttClients = [...mqttClients, res];
				onSuccess('MQTT client created.');
			}
			closeMqttModal();
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to save MQTT client');
		}
	}

	function requestDeleteMqtt(client: MqttClientResponse) {
		mqttDeleteConfirm = { id: client.id, url: client.url };
	}

	function connectionLabel(status: MqttConnectionStatus): string {
		switch (status) {
			case 'online':
				return 'Online';
			case 'connecting':
				return 'Connecting';
			case 'offline':
				return 'Offline';
		}
	}

	function connectionColor(status: MqttConnectionStatus): string {
		switch (status) {
			case 'online':
				return 'bg-success-500 dark:bg-success-400';
			case 'connecting':
				return 'bg-warning-500 dark:bg-warning-400';
			case 'offline':
				return 'bg-error-500 dark:bg-error-400';
		}
	}

	async function executeDeleteMqtt() {
		if (!mqttDeleteConfirm) return;
		const { id } = mqttDeleteConfirm;
		mqttDeleteConfirm = null;
		try {
			await deleteMqttClient(id);
			mqttClients = mqttClients.filter((c) => c.id !== id);
			onSuccess('MQTT client deleted.');
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to delete MQTT client');
		}
	}
</script>

<svelte:window
	onkeydown={(e) => {
		if (e.key === 'Escape') {
			if (showMqttModal) closeMqttModal();
			else if (mqttDeleteConfirm) mqttDeleteConfirm = null;
		}
	}}
/>

<div class="card mb-6 p-6">
	<div class="mb-4 flex items-center justify-between">
		<h2 class="h3">MQTT Clients</h2>
		<button class="btn preset-filled-primary-500" onclick={openCreateMqtt}> Add Client </button>
	</div>
	{#if clients === undefined}
		<p class="text-surface-600 dark:text-surface-400">Loading...</p>
	{:else}
		<p class="mb-4 text-surface-600 dark:text-surface-400">
			Configure MQTT broker connections for Home Assistant integration. Use a URL like <code>mqtt://broker:1883</code>
			or
			<code>mqtts://broker:8883</code>.
		</p>

		{#if mqttClients.length === 0}
			<p class="py-4 text-center text-surface-600 dark:text-surface-400">No MQTT clients configured.</p>
		{:else}
			<div class="table-wrap">
				<table class="table">
					<thead>
						<tr>
							<th>URL</th>
							<th>Client ID</th>
							<th>Topic Prefix</th>
							<th>HA Discovery</th>
							<th>Status</th>
							<th class="w-48">Actions</th>
						</tr>
					</thead>
					<tbody>
						{#each mqttClients as client (client.id)}
							<tr>
								<td>{client.url}</td>
								<td>{client.client_id}</td>
								<td>{client.topic_prefix}</td>
								<td>
									{#if client.ha_discovery}
										<span class="badge preset-filled-success-500">Enabled</span>
									{:else}
										<span class="badge preset-tonal">Disabled</span>
									{/if}
								</td>
								<td>
									{#if client.enabled}
										{@const connectionText = connectionLabel(client.connection_status)}
										<span class="inline-flex items-center gap-2">
											<span
												class={`h-2.5 w-2.5 rounded-full ${connectionColor(client.connection_status)}`}
												title={connectionText}
												aria-label={connectionText}
											></span>
											<span class="badge preset-filled-success-500">Enabled</span>
										</span>
									{:else}
										<span class="inline-flex items-center gap-2">
											<span
												class="h-2.5 w-2.5 rounded-full bg-surface-400 dark:bg-surface-600"
												title="Disabled"
												aria-label="Disabled"
											></span>
											<span class="badge preset-tonal">Disabled</span>
										</span>
									{/if}
								</td>
								<td>
									<div class="flex gap-1">
										<button class="btn btn-sm preset-tonal" onclick={() => openEditMqtt(client)}> Edit </button>
										<button class="btn btn-sm preset-tonal-error" onclick={() => requestDeleteMqtt(client)}>
											Delete
										</button>
									</div>
								</td>
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
		{/if}
	{/if}

	{#if mqttLimit !== null}
		<div class="mt-4 flex items-center gap-4 border-t border-surface-200 dark:border-surface-700 pt-4">
			<span class="text-surface-600 dark:text-surface-400">Max clients per tenant:</span>
			{#if editingLimit}
				<input class="input w-24" type="number" min="1" bind:value={limitInput} />
				<button class="btn btn-sm preset-filled-primary-500" onclick={saveMqttLimit} disabled={savingLimit}>
					{savingLimit ? 'Saving...' : 'Save'}
				</button>
				<button class="btn btn-sm preset-tonal-surface" onclick={() => (editingLimit = false)}>Cancel</button>
			{:else}
				<span class="font-medium">{mqttLimit}</span>
				<button
					class="btn btn-sm preset-tonal"
					onclick={() => {
						limitInput = mqttLimit!;
						editingLimit = true;
					}}>Edit</button
				>
			{/if}
		</div>
	{/if}
</div>

{#if mqttDeleteConfirm}
	<ConfirmDialog
		title="Delete MQTT Client"
		messagePrefix="Are you sure you want to delete the MQTT client"
		entityName={mqttDeleteConfirm.url}
		confirmLabel="Delete"
		onconfirm={executeDeleteMqtt}
		oncancel={() => {
			mqttDeleteConfirm = null;
		}}
	/>
{/if}

{#if showMqttModal}
	<ModalBackdrop onclose={closeMqttModal}>
		<div
			class="card bg-surface-50 dark:bg-surface-900 w-full max-w-2xl max-h-[90vh] space-y-4 overflow-y-auto p-6 shadow-xl"
			role="dialog"
			aria-modal="true"
		>
			<h3 class="h3">{editingMqttClient ? 'Edit MQTT Client' : 'Add MQTT Client'}</h3>

			<label class="flex items-center gap-3">
				<input class="checkbox" type="checkbox" bind:checked={mqttForm.enabled} />
				<span>Enabled</span>
			</label>

			<label class="label">
				<span>Broker URL</span>
				<input class="input" type="text" placeholder="e.g. mqtt://broker:1883" bind:value={mqttForm.url} />
			</label>

			<div class="grid grid-cols-1 gap-4 sm:grid-cols-2">
				<label class="label">
					<span>Client ID</span>
					<input class="input" type="text" bind:value={mqttForm.client_id} />
				</label>
				<label class="label">
					<span>Topic Prefix</span>
					<input class="input" type="text" bind:value={mqttForm.topic_prefix} />
				</label>
			</div>

			<div class="grid grid-cols-1 gap-4 sm:grid-cols-2">
				<label class="label">
					<span>Username</span>
					<input class="input" type="text" placeholder="(optional)" bind:value={mqttForm.username} />
				</label>
				<label class="label">
					<span>
						Password
						<!-- has_password (not has_client_secret) matches the backend MqttClientResponse type,
						     which uses the MQTT-idiomatic term "password" rather than the OAuth term "secret". -->
						{#if editingMqttClient?.has_password}
							<span class="badge preset-filled-success-500 ml-2 text-xs">Password set</span>
						{/if}
					</span>
					<input
						class="input"
						type="password"
						placeholder={editingMqttClient ? 'Leave blank to keep current' : '(optional)'}
						bind:value={mqttForm.password}
					/>
				</label>
			</div>

			<label class="label">
				<span>
					CA Certificate (PEM)
					{#if editingMqttClient?.has_ca_cert}
						<span class="badge preset-filled-success-500 ml-2 text-xs">CA cert set</span>
					{/if}
				</span>
				<textarea
					class="textarea font-mono text-sm"
					rows="4"
					placeholder={editingMqttClient
						? 'Leave blank to keep current'
						: '(optional) Paste PEM-encoded CA certificate for private brokers'}
					bind:value={mqttForm.ca_pem}
				></textarea>
			</label>

			<div class="rounded-container-token border border-surface-200 p-4 dark:border-surface-700">
				<p class="mb-3 font-medium">Home Assistant Integration</p>
				<label class="flex cursor-pointer items-center gap-3">
					<input class="checkbox" type="checkbox" bind:checked={mqttForm.ha_discovery} />
					<span>Enable Home Assistant Discovery</span>
				</label>
				{#if mqttForm.ha_discovery}
					<label class="label mt-3">
						<span>Discovery Prefix</span>
						<input class="input" type="text" placeholder="homeassistant" bind:value={mqttForm.ha_discovery_prefix} />
						<p class="text-surface-500 dark:text-surface-400 mt-1 text-sm">
							Must match the MQTT discovery prefix configured in Home Assistant (default:
							<code>homeassistant</code>).
						</p>
					</label>
				{/if}
			</div>

			<div class="flex justify-end gap-2 items-center">
				{#if !getIsOnline()}<span class="text-warning-500 text-sm mr-auto">Offline</span>{/if}
				<button class="btn preset-tonal-surface" onclick={closeMqttModal}>Cancel</button>
				<button class="btn preset-filled-primary-500" onclick={saveMqttClient} disabled={!getIsOnline()}>
					{editingMqttClient ? 'Update' : 'Create'}
				</button>
			</div>
		</div>
	</ModalBackdrop>
{/if}
