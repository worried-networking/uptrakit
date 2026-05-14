<script lang="ts">
	import { SectionCard } from '$lib/components/ui';
	import { computeDiff, type DiffEntry } from './diff';

	type Props = {
		before: Record<string, unknown> | null;
		after: Record<string, unknown> | null;
	};
	let { before, after }: Props = $props();

	let rows = $derived(computeDiff(before, after));

	function renderValue(v: unknown): string {
		if (v === undefined) return '—';
		if (v === null) return 'null';
		if (typeof v === 'string') return v;
		return JSON.stringify(v);
	}

	const statusToToken: Record<DiffEntry['status'], string> = {
		unchanged: 'var(--text-muted)',
		changed: 'var(--color-warning)',
		added: 'var(--color-success)',
		removed: 'var(--color-danger)'
	};
</script>

<SectionCard title="State Changes">
	<table class="w-full text-sm">
		<thead>
			<tr class="border-b border-[var(--border-subtle)] text-left text-[var(--text-secondary)]">
				<th class="table-cell-pad font-semibold uppercase tracking-table-header text-table-header">Key</th>
				<th class="table-cell-pad font-semibold uppercase tracking-table-header text-table-header">Before</th>
				<th class="table-cell-pad font-semibold uppercase tracking-table-header text-table-header">After</th>
			</tr>
		</thead>
		<tbody>
			{#each rows as row (row.key)}
				<tr class="border-b border-[var(--border-subtle)] last:border-b-0" style:color={statusToToken[row.status]}>
					<td class="table-cell-pad font-mono">{row.key}</td>
					<td class="table-cell-pad">{renderValue(row.before)}</td>
					<td class="table-cell-pad">{renderValue(row.after)}</td>
				</tr>
			{/each}
		</tbody>
	</table>
</SectionCard>
