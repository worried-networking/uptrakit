<script lang="ts">
	import { page } from '$app/stores';
	import { goto } from '$app/navigation';
</script>

<div class="container h-full flex flex-col items-center justify-center text-center">
	<h1 class="h1 mb-4">{$page.status}: {$page.error?.message}</h1>
	<p class="text-lg text-surface-500 mb-8">An unexpected error occurred.</p>
	<button class="btn preset-filled-primary-500" onclick={() => goto('/')}>Go to Home</button>
</div>