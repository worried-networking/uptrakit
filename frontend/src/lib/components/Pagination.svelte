<script lang="ts">
	let {
		currentPage,
		totalPages,
		onPageChange
	}: {
		currentPage: number;
		totalPages: number;
		onPageChange: (page: number) => void;
	} = $props();
</script>

{#if totalPages > 1}
	<nav class="mt-4 flex items-center justify-center gap-2" aria-label="Pagination">
		<button
			class="btn btn-sm preset-tonal"
			disabled={currentPage <= 1}
			onclick={() => onPageChange(currentPage - 1)}
		>
			Previous
		</button>
		<span class="text-sm text-surface-600 dark:text-surface-400">
			Page {currentPage} of {totalPages}
		</span>
		<button
			class="btn btn-sm preset-tonal"
			disabled={currentPage >= totalPages}
			onclick={() => onPageChange(currentPage + 1)}
		>
			Next
		</button>
	</nav>
{/if}
