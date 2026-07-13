import { SvelteURL } from 'svelte/reactivity';

export const page = $state({
	url: new SvelteURL('http://localhost/'),
	params: {} as Record<string, string>
});
