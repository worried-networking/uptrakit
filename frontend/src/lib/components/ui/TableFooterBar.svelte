<script lang="ts">
	import Pagination from '../Pagination.svelte';

	let {
		total,
		currentPage,
		totalPages,
		onPageChange
	}: {
		total: number;
		currentPage: number;
		totalPages: number;
		onPageChange: (page: number) => void;
	} = $props();
</script>

<div
	class="flex items-center justify-between border-t border-[var(--border-subtle)] bg-[var(--bg-surface)] px-4 py-3"
	data-ui="table-footer-bar"
>
	<span class="text-table-body text-[var(--text-secondary)]">{total} total</span>
	<Pagination {currentPage} {totalPages} {onPageChange} />
</div>
