<script lang="ts" module>
	let _capturedOnPageChange: ((dataSourceId: string, page: number) => void) | undefined;

	export function getCapturedOnPageChange() {
		return _capturedOnPageChange;
	}

	export function resetCapturedOnPageChange() {
		_capturedOnPageChange = undefined;
	}
</script>

<script lang="ts">
	import type { SurfaceReadResponse, SurfaceResponse } from '$lib/surfaces/contract';

	let {
		onPageChange
	}: {
		surface?: SurfaceResponse;
		read?: SurfaceReadResponse;
		baseParams?: Record<string, unknown>;
		reloadToken?: string | number;
		pageBySource?: Record<string, number>;
		onPageChange?: (dataSourceId: string, page: number) => void;
	} = $props();

	$effect(() => {
		_capturedOnPageChange = onPageChange;
	});
</script>

<div data-testid="surface-read-panel-stub"></div>
