<script lang="ts">
	import { user, handleRegister } from '$lib/auth';
	import { goto } from '$app/navigation';

	let email = $state('');
	let firstName = $state('');
	let lastName = $state('');
	let password = $state('');
	let showToken = $state(false);
	let registrationToken = $state('');
	let error = $state('');

	$effect(() => {
		if ($user) {
			goto('/');
		}
	});

	async function onSubmit(e: SubmitEvent) {
		e.preventDefault();
		error = '';
		try {
			await handleRegister({
				email,
				first_name: firstName,
				last_name: lastName,
				password,
				...(showToken && registrationToken ? { registration_token: registrationToken } : {})
			});
			goto('/');
		} catch (err) {
			error = err instanceof Error ? err.message : 'Registration failed';
		}
	}
</script>

<div class="card mx-auto mt-8 max-w-md p-8">
	<h2 class="h2 mb-6 text-center">Register</h2>

	{#if error}
		<aside class="alert variant-filled-error mb-4">
			<div class="alert-message">
				<p>{error}</p>
			</div>
		</aside>
	{/if}

	<form onsubmit={onSubmit} class="space-y-4">
		<label class="label">
			<span>Email</span>
			<input
				class="input"
				type="email"
				bind:value={email}
				required
				autocomplete="email"
			/>
		</label>

		<label class="label">
			<span>First name</span>
			<input
				class="input"
				type="text"
				bind:value={firstName}
				required
				autocomplete="given-name"
			/>
		</label>

		<label class="label">
			<span>Last name</span>
			<input
				class="input"
				type="text"
				bind:value={lastName}
				required
				autocomplete="family-name"
			/>
		</label>

		<label class="label">
			<span>Password</span>
			<input
				class="input"
				type="password"
				bind:value={password}
				required
				minlength={8}
				autocomplete="new-password"
			/>
		</label>

		<label class="flex items-center space-x-2">
			<input class="checkbox" type="checkbox" bind:checked={showToken} />
			<span>I have an invite token</span>
		</label>

		{#if showToken}
			<label class="label">
				<span>Invite token</span>
				<input
					class="input"
					type="text"
					bind:value={registrationToken}
					autocomplete="off"
				/>
			</label>
		{/if}

		<button type="submit" class="btn variant-filled-primary w-full">Register</button>
	</form>

	<p class="mt-4 text-center">
		Already have an account? <a href="/login" class="anchor">Login</a>
	</p>
</div>
