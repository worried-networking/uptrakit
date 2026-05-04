import type { ComponentType, SvelteComponent } from 'svelte';
import {
	Box,
	Boxes,
	Check,
	Cpu,
	Database,
	FileText,
	Globe,
	HardDrive,
	History,
	Layers,
	Link,
	Package,
	PlugZap,
	Puzzle,
	Radar,
	RefreshCw,
	ScrollText,
	Server,
	ServerCog,
	Settings,
	Shield,
	Tag,
	Tags,
	Trash2,
	Unlink,
	Wrench
} from 'lucide-svelte';

export type IconComponent = ComponentType<SvelteComponent>;

export const ICONS: Record<string, IconComponent> = {
	box: Box,
	boxes: Boxes,
	check: Check,
	cpu: Cpu,
	database: Database,
	'file-text': FileText,
	globe: Globe,
	'hard-drive': HardDrive,
	history: History,
	layers: Layers,
	link: Link,
	package: Package,
	'plug-zap': PlugZap,
	puzzle: Puzzle,
	radar: Radar,
	'refresh-cw': RefreshCw,
	'scroll-text': ScrollText,
	server: Server,
	'server-cog': ServerCog,
	settings: Settings,
	shield: Shield,
	tag: Tag,
	tags: Tags,
	'trash-2': Trash2,
	unlink: Unlink,
	wrench: Wrench
};

export interface ResolvedIcon {
	component: IconComponent;
	ok: boolean;
}

const FALLBACK: IconComponent = Box;

export function resolveIcon(name: string | null | undefined): ResolvedIcon {
	if (!name) {
		return { component: FALLBACK, ok: false };
	}
	const component = ICONS[name];
	if (!component) {
		console.error(`[surfaces] Unknown icon name: "${name}"`);
		return { component: FALLBACK, ok: false };
	}
	return { component, ok: true };
}
