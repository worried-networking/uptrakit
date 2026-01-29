<script lang="ts">
	import { user, handleLogin } from '$lib/auth';
	import { goto } from '$app/navigation';

	let email = $state('');
	let password = $state('');
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
			await handleLogin({ email, password });
			goto('/');
		} catch (err) {
			error = err instanceof Error ? err.message : 'Login failed';
		}
	}
</script>

<div class="card mx-auto mt-8 max-w-md p-8">
	<h2 class="h2 mb-6 text-center">Login</h2>

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
			<span>Password</span>
			<input
				class="input"
				type="password"
				bind:value={password}
				required
				autocomplete="current-password"
			/>
		</label>

		<button type="submit" class="btn variant-filled-primary w-full">Login</button>
	</form>

	<p class="mt-4 text-center">
		Don't have an account? <a href="/register" class="anchor">Register</a>
	</p>
</div>
