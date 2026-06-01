<script lang="ts">
	import { untrack } from 'svelte';
	import { createUrlParam, type UrlParamOptions } from './url-params.svelte';

	let {
		paramKey,
		parse,
		serialize,
		testSetValue = ''
	}: {
		paramKey: string;
		parse?: (r: string | null) => unknown;
		serialize?: (v: unknown) => string | null;
		testSetValue?: unknown;
	} = $props();

	// createUrlParam captures its arguments at init scope by design; untrack
	// the prop reads so re-renders of testSetValue don't trip the
	// state_referenced_locally warning.
	const param = untrack(() =>
		// eslint-disable-next-line @typescript-eslint/no-explicit-any -- test harness requires generic param type
		createUrlParam(paramKey, { parse, serialize } as UrlParamOptions<any>)
	);
</script>

<span data-testid="current-value">{JSON.stringify(param.value)}</span>
<button data-testid="do-set" onclick={() => param.set(testSetValue)}>set</button>
