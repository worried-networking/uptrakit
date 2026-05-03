import { getContext, setContext } from 'svelte';

export enum FormLayout {
	Modal = 'modal',
	Page = 'page'
}

const KEY = 'uptrakit:form-layout';

export const LABEL_COL: Record<FormLayout, string> = {
	[FormLayout.Modal]: 'md:grid-cols-[minmax(0,11rem)_minmax(0,1fr)]',
	[FormLayout.Page]: 'md:grid-cols-[minmax(0,20rem)_minmax(0,1fr)]'
};

export function setFormLayout(layout: FormLayout): void {
	setContext(KEY, layout);
}

export function getFormLayout(): FormLayout {
	return getContext<FormLayout>(KEY) ?? FormLayout.Page;
}
