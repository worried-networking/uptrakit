export const LabelDisplay = {
	Always: 'always',
	Auto: 'auto',
	IconOnly: 'icon-only'
} as const;

export type LabelDisplay = (typeof LabelDisplay)[keyof typeof LabelDisplay];
