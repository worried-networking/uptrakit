// Reparents host element to <body> on mount so position: fixed escapes any
// ancestor containing block (e.g. contain: layout on <main>). Action setup
// runs after DOM insertion and before component $effect callbacks, so
// positioning logic that reads getBoundingClientRect() sees the portaled
// position on first run.
export function portal(node: HTMLElement) {
	document.body.appendChild(node);
	return {
		destroy() {
			node.parentNode?.removeChild(node);
		}
	};
}
