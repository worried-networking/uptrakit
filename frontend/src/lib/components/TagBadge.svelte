<script lang="ts">
	let { name, color }: { name: string; color: string } = $props();

	/**
	 * Determines whether white or dark text provides better contrast
	 * against the given hex background color using relative luminance.
	 */
	function textColor(hex: string): string {
		const c = hex.replace('#', '');
		const r = parseInt(c.substring(0, 2), 16) / 255;
		const g = parseInt(c.substring(2, 4), 16) / 255;
		const b = parseInt(c.substring(4, 6), 16) / 255;
		// sRGB to linear
		const lr = r <= 0.03928 ? r / 12.92 : Math.pow((r + 0.055) / 1.055, 2.4);
		const lg = g <= 0.03928 ? g / 12.92 : Math.pow((g + 0.055) / 1.055, 2.4);
		const lb = b <= 0.03928 ? b / 12.92 : Math.pow((b + 0.055) / 1.055, 2.4);
		const luminance = 0.2126 * lr + 0.7152 * lg + 0.0722 * lb;
		return luminance > 0.179 ? '#1a1a1a' : '#ffffff';
	}
</script>

<span
	class="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium leading-tight"
	style="background-color: {color}; color: {textColor(color)}"
>
	{name}
</span>
