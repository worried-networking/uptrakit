import type { ComponentType, SvelteComponent } from 'svelte';
import {
	Box,
	Cpu,
	Database,
	FileText,
	Globe,
	HardDrive,
	History,
	Layers,
	Package,
	Puzzle,
	ScrollText,
	Server,
	ServerCog,
	Settings,
	Shield,
	Tag,
	Tags,
	Wrench
} from 'lucide-svelte';

export const SURFACE_NAV_ICONS: Record<string, ComponentType<SvelteComponent>> = {
	Box,
	Cpu,
	Database,
	FileText,
	Globe,
	HardDrive,
	History,
	Layers,
	Package,
	Puzzle,
	ScrollText,
	Server,
	ServerCog,
	Settings,
	Shield,
	Tag,
	Tags,
	Wrench
};

export function resolveNavIcon(name: string): ComponentType<SvelteComponent> {
	return SURFACE_NAV_ICONS[name] ?? Box;
}
