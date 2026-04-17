<script lang="ts">
	let {
		currentPage,
		totalPages,
		total: _total = undefined,
		onPageChange
	}: {
		currentPage: number;
		totalPages: number;
		total?: number;
		onPageChange: (page: number) => void;
	} = $props();

	/**
	 * Build a list of page numbers and ellipsis markers to render.
	 *
	 * Strategy:
	 *  - Always show first and last page
	 *  - Always show current page and 2 neighbours on each side
	 *  - Bridge gaps with `null` (rendered as "...")
	 *  - For <= 7 total pages, show all without ellipsis
	 */
	function visiblePages(current: number, last: number): (number | null)[] {
		if (last <= 7) {
			return Array.from({ length: last }, (_, i) => i + 1);
		}

		// Collect candidate pages, then deduplicate and sort.
		const candidates = [1, last, current - 2, current - 1, current, current + 1, current + 2].filter(
			(p) => p >= 1 && p <= last
		);

		const sorted = candidates.sort((a, b) => a - b).filter((p, i, arr) => i === 0 || p !== arr[i - 1]);
		const result: (number | null)[] = [];

		for (let i = 0; i < sorted.length; i++) {
			if (i > 0 && sorted[i] - sorted[i - 1] > 1) {
				result.push(null);
			}
			result.push(sorted[i]);
		}

		return result;
	}

	let pages = $derived(visiblePages(currentPage, totalPages));
</script>

{#if totalPages > 1}
	<div class="flex items-center justify-end">
		<nav class="flex items-center gap-1" aria-label="Pagination">
			<button
				class="btn btn-sm preset-tonal h-8 min-h-8 px-3 text-[10px]"
				disabled={currentPage <= 1}
				onclick={() => onPageChange(currentPage - 1)}
			>
				Previous
			</button>
			{#each pages as p, idx (p ?? `ellipsis-${idx}`)}
				{#if p === null}
					<span class="px-1 text-[10px] text-[var(--text-secondary)]">&hellip;</span>
				{:else}
					<button
						class={`btn btn-sm h-8 min-h-8 min-w-8 px-2.5 text-[10px] ${
							p === currentPage ? 'preset-filled-primary-500' : 'preset-tonal'
						}`}
						onclick={() => onPageChange(p)}
						aria-current={p === currentPage ? 'page' : undefined}
					>
						{p}
					</button>
				{/if}
			{/each}
			<button
				class="btn btn-sm preset-tonal h-8 min-h-8 px-3 text-[10px]"
				disabled={currentPage >= totalPages}
				onclick={() => onPageChange(currentPage + 1)}
			>
				Next
			</button>
		</nav>
	</div>
{/if}
