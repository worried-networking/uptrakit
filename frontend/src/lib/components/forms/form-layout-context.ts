import { getContext, setContext } from 'svelte';

export enum FormLayout {
	Modal = 'modal',
	Page = 'page'
}

const KEY = 'uptrakit:form-layout';

export const LABEL_COL: Record<FormLayout, string> = {
	// 24rem: modal half-cell ≈ 19rem < 24rem (stays stacked); full-width modal ≈ 39rem > 24rem (side-by-side)
	[FormLayout.Modal]: '@[24rem]:grid-cols-[minmax(0,11rem)_minmax(0,1fr)] @[24rem]:items-start',
	// 32rem: page content areas are typically 37rem+ at desktop widths
	[FormLayout.Page]: '@[32rem]:grid-cols-[minmax(0,20rem)_minmax(0,1fr)] @[32rem]:items-start'
};

export function setFormLayout(layout: FormLayout): void {
	setContext(KEY, layout);
}

export function getFormLayout(): FormLayout {
	return getContext<FormLayout>(KEY) ?? FormLayout.Page;
}
