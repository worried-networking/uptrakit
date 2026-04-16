import type { FormField } from '$lib/types';

export type SurfaceId = string;
export type InteractionId = string;
export type DataSourceId = string;
export type ControllerQueryId = string;
export type BuiltInApiOperationId = string;
export type SurfaceTabId = string;

export type SurfaceTargeting = 'universal' | 'targeted';
export type SurfaceScope = 'global' | 'tenant';
export type SurfaceProviderKind = 'built_in' | 'plugin' | 'service';

export type SurfaceCapability =
	| 'section_node'
	| 'text_block_node'
	| 'key_value_node'
	| 'table_node'
	| 'form_node'
	| 'action_bar_node'
	| 'tabs_node'
	| 'callout_node'
	| 'empty_state_node'
	| 'modal_trigger_node'
	| 'workflow_trigger_node'
	| 'mutation_action'
	| 'form_submit'
	| 'workflow'
	| 'navigate'
	| 'data_load'
	| 'confirmable_action'
	| 'static_data_source'
	| 'controller_query_data_source'
	| 'provider_query_data_source'
	| 'universal_targeting'
	| 'targeted_targeting'
	| 'sensitive_fields'
	| 'provider_initiated_actions';

export type SchemaContract = 'any' | 'object' | 'array' | 'string' | 'integer' | 'number' | 'boolean' | 'null';

export interface SurfaceTab {
	id: SurfaceTabId;
	label: string;
	root: SurfaceNode;
}

export interface SurfaceTableColumn {
	key: string;
	label: string;
}

export interface SurfaceRowVisibleWhen {
	field: string;
	condition: 'present' | 'absent';
}

export interface SurfaceTableRowAction {
	interaction_id: InteractionId;
	visible_when?: SurfaceRowVisibleWhen;
}

export type SurfaceNode =
	| {
			kind: 'section';
			title?: string;
			children?: SurfaceNode[];
	  }
	| {
			kind: 'text_block';
			text: string;
	  }
	| {
			kind: 'key_value';
			data_source_id: DataSourceId;
	  }
	| {
			kind: 'table';
			data_source_id: DataSourceId;
			columns?: SurfaceTableColumn[];
			row_actions?: SurfaceTableRowAction[];
	  }
	| {
			kind: 'form';
			interaction_id: InteractionId;
	  }
	| {
			kind: 'action_bar';
			action_ids?: InteractionId[];
	  }
	| {
			kind: 'tabs';
			tabs?: SurfaceTab[];
	  }
	| {
			kind: 'callout';
			level: 'info' | 'warning' | 'danger';
			text: string;
	  }
	| {
			kind: 'empty_state';
			title: string;
			description?: string;
	  }
	| {
			kind: 'modal_trigger';
			interaction_id: InteractionId;
			modal_nodes?: SurfaceNode[];
	  }
	| {
			kind: 'workflow_trigger';
			interaction_id: InteractionId;
			step_nodes?: SurfaceNode[];
	  };

export interface SurfaceDescriptor {
	surface_id: SurfaceId;
	label: string;
	priority: number;
	slot: string;
	scope: SurfaceScope;
	targeting: SurfaceTargeting;
	required_permission?: string;
	provider_kind: SurfaceProviderKind;
	required_capabilities: SurfaceCapability[];
	root_node: SurfaceNode;
}

export type DataSourceKind =
	| { kind: 'static'; data: unknown }
	| { kind: 'controller_query'; query_id: ControllerQueryId }
	| { kind: 'provider_query'; operation_id: string };

export type RefreshPolicy = { type: 'manual' } | { type: 'interval'; seconds: number } | { type: 'sse'; topic: string };

export interface DataSourcePagination {
	default_page_size: number;
	max_page_size: number;
}

export interface DataSourceSorting {
	sortable_fields?: string[];
	default_sort_field?: string;
}

export interface DataSourceFiltering {
	filter_fields?: string[];
}

export interface DataSourceEmptyState {
	title: string;
	description?: string;
}

export interface DataSourceDescriptor {
	data_source_id: DataSourceId;
	kind: DataSourceKind;
	result_schema: SchemaContract;
	pagination?: DataSourcePagination;
	sorting?: DataSourceSorting;
	filtering?: DataSourceFiltering;
	refresh_policy: RefreshPolicy;
	empty_state?: DataSourceEmptyState;
}

export type InteractionKind =
	| 'mutation_action'
	| 'form_submit'
	| 'workflow'
	| 'navigate'
	| 'data_load'
	| 'confirmable_action';

export type InteractionTransport =
	| { mode: 'controller_local' }
	| { mode: 'provider_proxied' }
	| { mode: 'direct_built_in_api'; operation_id: BuiltInApiOperationId };

export interface InteractionConfirmation {
	title: string;
	message: string;
	confirm_label?: string;
	cancel_label?: string;
	severity: 'info' | 'warning' | 'danger';
}

export interface WorkflowStepDescriptor {
	step_id: string;
	label?: string;
	form_ui?: FormUiDescriptor;
	submit_interaction_id?: InteractionId;
	render_previous_response?: boolean;
	input_schema: SchemaContract;
	result_schema: SchemaContract;
}

export interface FormUiDescriptor {
	fields: FormField[];
	pre_load_interaction_id?: InteractionId;
}

export interface InteractionDescriptor {
	interaction_id: InteractionId;
	kind: InteractionKind;
	label?: string;
	required_permission?: string;
	input_schema?: SchemaContract;
	result_schema?: SchemaContract;
	sensitive_fields?: string[];
	timeout_seconds?: number;
	confirmation?: InteractionConfirmation;
	transport: InteractionTransport;
	workflow_steps?: WorkflowStepDescriptor[];
	form_ui?: FormUiDescriptor;
}

export interface RegisteredSurface {
	descriptor: SurfaceDescriptor;
	interactions?: InteractionDescriptor[];
	data_sources?: DataSourceDescriptor[];
}

export interface SurfaceResponse extends SurfaceDescriptor {
	provider_count: number;
}

export type SurfaceProviderAvailability = 'available' | 'disconnected' | 'incompatible_tenant';

export interface ProviderEncryptionMetadata {
	key_id: string;
	algorithm: 'ecies_p256';
	public_key: string;
}

export interface SurfaceProviderInfo {
	provider_id: string;
	display_label: string;
	service_id?: string;
	availability: SurfaceProviderAvailability;
	encryption_metadata?: ProviderEncryptionMetadata;
}

export interface SurfaceRuntimeStatusResponse {
	active: boolean;
}

export interface SurfaceReadResponse {
	descriptor: SurfaceDescriptor;
	interactions: InteractionDescriptor[];
	data_sources: DataSourceDescriptor[];
}

export interface InvokeSurfaceInteractionRequest {
	params?: Record<string, unknown>;
	encrypted_sensitive_params?: {
		key_id: string;
		algorithm: 'ecies_p256';
		ciphertext_b64: string;
	};
	target_provider_id?: string;
	idempotency_key?: string;
	timeout_seconds?: number;
}
