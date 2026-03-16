export interface ErrorResponse {
	error: string;
	code?: string;
}

export enum Permission {
	// Services
	ViewServices = 'view_services',
	ApproveServices = 'approve_services',
	RejectServices = 'reject_services',
	RemoveServices = 'remove_services',
	UpdateServices = 'update_services',
	// System services
	ViewSystemServices = 'view_system_services',
	ApproveSystemServices = 'approve_system_services',
	RejectSystemServices = 'reject_system_services',
	RemoveSystemServices = 'remove_system_services',
	UpdateSystemServices = 'update_system_services',
	// Software
	ViewSoftware = 'view_software',
	CreateSoftware = 'create_software',
	UpdateSoftware = 'update_software',
	DeleteSoftware = 'delete_software',
	TriggerChecks = 'trigger_checks',
	TriggerUpdates = 'trigger_updates',
	ManageScheduler = 'manage_scheduler',
	// Hosts
	ViewHosts = 'view_hosts',
	UpdateHosts = 'update_hosts',
	DeactivateHosts = 'deactivate_hosts',
	// Settings
	ViewSettings = 'view_settings',
	ManageAuthSettings = 'manage_auth_settings',
	ManageEnrollmentTokens = 'manage_enrollment_tokens',
	ManageAgentCerts = 'manage_agent_certs',
	ManageGlobalSettings = 'manage_global_settings',
	// Commands
	ManageCommands = 'manage_commands',
	// Notifications
	ViewNotifications = 'view_notifications',
	ManageNotifications = 'manage_notifications',
	// Audit logs
	ViewAuditLogs = 'view_audit_logs',
	ViewSystemAuditLogs = 'view_system_audit_logs',
	// Users & ignores
	ManageUsers = 'manage_users',
	ManageIgnores = 'manage_ignores'
}

/** Returns true if the user holds at least one of the given permissions. */
export function hasAnyPermission(user: User | null | undefined, ...perms: Permission[]): boolean {
	if (!user) return false;
	return perms.some((p) => user.permissions.includes(p));
}

export function hasPermissionValue(user: User | null | undefined, permission?: string | null): boolean {
	if (!permission) return true;
	if (!user) return false;
	return user.permissions.includes(permission as Permission);
}

export interface User {
	id: string;
	email: string;
	first_name: string;
	last_name: string;
	permissions: Permission[];
}

export interface AuthResponse {
	access_token: string;
	refresh_token: string;
	expires_in: number;
	token_type: string;
	user: User;
}

export interface RefreshResponse {
	access_token: string;
	refresh_token: string;
	expires_in: number;
	token_type: string;
}

export interface RegisterRequest {
	email: string;
	first_name: string;
	last_name: string;
	password: string;
	registration_token?: string;
}

export interface LoginRequest {
	email: string;
	password: string;
}

export type ServiceStatus = 'pending' | 'approved' | 'rejected' | 'deactivated';

export interface ServiceResponse {
	id: string;
	capabilities: string[];
	service_label: string;
	hostname: string;
	friendly_name: string;
	ip_address: string | null;
	status: ServiceStatus;
	client_version: string | null;
	last_seen_at: string | null;
	created_at: string;
	updated_at: string;
	ping_interval_seconds?: number | null;
}

export interface UpdateServiceRequest {
	ping_interval_seconds?: number;
}

export interface MessageResponse {
	message: string;
}

export interface OidcProviderInfo {
	id: string;
	name: string;
	slug: string;
	logo_url?: string;
}

export interface AuthMethodsResponse {
	password: boolean;
	oidc_providers: OidcProviderInfo[];
	setup_required: boolean;
	registration_token_required: boolean;
}

export interface OidcLinkRequest {
	link_token: string;
	password?: string;
}

export interface RegistrationSettings {
	mode: 'open' | 'invite' | 'closed';
	require_token_for_oidc: boolean;
}

export interface UpdateRegistrationSettings {
	mode: 'open' | 'invite' | 'closed';
	token?: string;
	require_token_for_oidc?: boolean;
}

export interface AuthenticationSettings {
	password_auth_enabled: boolean;
}

export interface UpdateAuthenticationSettings {
	password_auth_enabled?: boolean;
}

export interface AgentCertificateSettings {
	lifetime_days: number;
	/** Admin-configured override. `null` means automatic mode. */
	renewal_window_hours_override: number | null;
	/** Effective renewal window in hours. In auto mode: min(14 days, lifetime/5). */
	effective_renewal_window_hours: number;
}

export interface UpdateAgentCertificateSettings {
	lifetime_days?: number;
	/** Set to `0` to reset to automatic mode (min(14 days, lifetime/5)). */
	renewal_window_hours?: number;
}

export interface CreateEnrollmentTokenRequest {
	name: string;
	allowed_capabilities?: string[];
	max_uses?: number;
	expires_in_seconds?: number;
}

export interface EnrollmentTokenCreatedResponse {
	id: string;
	token: string;
	name: string;
	allowed_capabilities: string[] | null;
	max_uses: number | null;
	current_uses: number;
	expires_at: string | null;
	created_at: string;
	created_by_user_id: string | null;
}

export interface EnrollmentTokenResponse {
	id: string;
	name: string;
	allowed_capabilities: string[] | null;
	max_uses: number | null;
	current_uses: number;
	expires_at: string | null;
	created_at: string;
	revoked_at: string | null;
	created_by_user_id: string | null;
}

export interface EnrollmentTokensSummary {
	active_count: number;
}

export interface CombinedSettingsResponse {
	registration: RegistrationSettings;
	authentication: AuthenticationSettings;
	agent_certificates: AgentCertificateSettings;
	enrollment_tokens: EnrollmentTokensSummary;
}

export interface OidcProviderResponse {
	id: string;
	name: string;
	slug: string;
	logo_url: string | null;
	issuer_url: string;
	client_id: string;
	has_client_secret: boolean;
	scopes: string;
	auto_create_users: boolean;
	role_claim_path: string | null;
	role_mapping: Record<string, string>;
	is_active: boolean;
	created_at: string;
	updated_at: string;
}

export interface CreateOidcProviderRequest {
	name: string;
	slug: string;
	issuer_url: string;
	client_id: string;
	client_secret: string;
	logo_url?: string;
	scopes?: string;
	auto_create_users?: boolean;
	role_claim_path?: string;
	role_mapping?: Record<string, string>;
}

export interface SystemAlert {
	id: string;
	severity: 'info' | 'warning' | 'error' | 'critical';
	title: string;
	message: string;
	action?: string;
}

export interface SystemAlertsResponse {
	alerts: SystemAlert[];
}

export interface RenewServerCertResponse {
	message: string;
}

export interface NetworkSettings {
	trusted_proxies: string[];
	real_ip_header: string;
	sans: string[];
	https_addr: string;
	cert_regenerated?: boolean;
}

export interface UpdateNetworkSettings {
	trusted_proxies?: string[];
	real_ip_header?: string;
	sans?: string[];
	https_addr?: string;
	regenerate_cert?: boolean;
}

export interface HostAgentSummary {
	id: string;
	friendly_name: string;
	status: ServiceStatus;
}

export interface HostTagResponse {
	id: string;
	name: string;
	color: string;
	description: string | null;
	created_at: string;
	updated_at: string;
	host_count: number;
}

export interface HostTagSummary {
	id: string;
	name: string;
	color: string;
}

export interface CreateHostTagRequest {
	name: string;
	color?: string;
	description?: string;
}

export interface UpdateHostTagRequest {
	name?: string;
	color?: string;
	description?: string | null;
}

export interface SetHostTagsRequest {
	tag_ids: string[];
}

export interface HostResponse {
	id: string;
	machine_id: string;
	hostname: string;
	friendly_name: string;
	os_type: string | null;
	os_version: string | null;
	architecture: string | null;
	ip_address: string | null;
	last_seen_at: string | null;
	created_at: string;
	updated_at: string;
	agents: HostAgentSummary[];
	tags: HostTagSummary[];
}

export interface UpdateHostRequest {
	friendly_name?: string;
}

export interface PaginatedResponse<T> {
	items: T[];
	total: number;
	page: number;
	per_page: number;
	total_pages: number;
}

export interface HostPluginRoleSummary {
	role: string;
	/** Ordinal for hook roles (0-based). Always 0 for non-hook roles. */
	ordinal: number;
	plugin_config_id: string | null;
	plugin_config_name: string | null;
	plugin_type: string;
	package_identifier: string;
	config_override: Record<string, unknown> | null;
	execution_site: string;
}

export interface HostPluginRoleAssignment {
	role: string;
	/** Ordinal for hook roles; must be 0 for non-hook roles. Defaults to 0. */
	ordinal?: number;
	plugin_config_id?: string;
	plugin_config?: CreatePluginConfigRequest;
	package_identifier?: string;
	config_override?: Record<string, unknown> | null;
	execution_site?: string;
}

export enum PluginCapability {
	DiscoverLocalSoftware = 'discover_local_software',
	RefreshPackageIndex = 'refresh_package_index',
	DetectHostCompatibility = 'detect_host_compatibility',
	ControllerSideFetchReleases = 'controller_side_fetch_releases',
	VersionDetection = 'version_detection',
	ReleaseFetching = 'release_fetching',
	UpdateExecution = 'update_execution',
	NotificationDelivery = 'notification_delivery',
	HostLifecycle = 'host_lifecycle',
	HostReport = 'host_report',
	GuestExec = 'guest_exec',
	ServiceMigrations = 'service_migrations',
	ControllerMigrations = 'controller_migrations',
	UpdateLifecycle = 'update_lifecycle'
}

/** Static metadata for a plugin type, returned by `GET /api/v1/plugin-types`. */
export interface PluginTypeInfo {
	/** Snake_case wire identifier, e.g. `"releases_github"`. */
	plugin_type: string;
	/** Human-readable display name, e.g. `"GitHub Releases"`. */
	display_name: string;
	/** Capabilities declared by this plugin type. */
	capabilities: PluginCapability[];
	/** Sample/default configuration JSON for this plugin type. */
	sample_config: Record<string, unknown>;
	/** Form field definitions for this plugin type. Empty for plugins with no configurable fields. */
	config_form_fields?: FieldDef[];
	/** Form field definitions for type-level settings. Empty for types with no configurable type settings. */
	type_settings_form_fields?: FieldDef[];
	/** Sample/default type settings JSON for this plugin type. */
	type_settings_sample?: Record<string, unknown>;
}

export interface PluginTypeSettingsResponse {
	plugin_type: string;
	config: Record<string, unknown>;
	created_at: string;
	updated_at: string;
}

export interface UpsertPluginTypeSettingsRequest {
	config: Record<string, unknown>;
}

export interface PluginConfigResponse {
	id: string;
	name: string;
	plugin_type: string;
	config: Record<string, unknown>;
	enabled: boolean;
	capabilities: PluginCapability[];
	created_at: string;
	updated_at: string;
}

export interface CreatePluginConfigRequest {
	name: string;
	plugin_type: string;
	config: Record<string, unknown>;
	enabled?: boolean;
}

export interface CreateSoftwareItemRequest {
	name: string;
	featured?: boolean;
	icon_url?: string | null;
}

export interface SoftwareItemResponse {
	id: string;
	name: string;
	plugins: string[];
	featured: boolean;
	last_checked_at: string | null;
	host_count: number;
	installed_version?: string | null;
	installed_display_version?: string | null;
	latest_version?: string | null;
	latest_release_metadata?: Record<string, unknown> | null;
	update_available: boolean;
	created_at: string;
	updated_at: string;
	icon_url?: string | null;
}

export interface SoftwareItemHostSummary {
	id: string;
	host_id: string;
	hostname: string;
	friendly_name: string;
	qualifier?: string | null;
	installed_version: string | null;
	installed_version_detected_at: string | null;
	installed_display_version?: string | null;
	latest_version?: string | null;
	latest_release_metadata?: Record<string, unknown> | null;
	update_available: boolean;
	active_update_history_id?: string | null;
	last_updated_at: string | null;
	linked_at: string;
	plugins: HostPluginRoleSummary[];
}

export interface SoftwareItemDetailResponse extends SoftwareItemResponse {
	hosts: SoftwareItemHostSummary[];
}

export interface HostSoftwareAssignment {
	host_id: string;
	plugins: HostPluginRoleAssignment[];
}

export interface AssignHostsRequest {
	host_assignments: HostSoftwareAssignment[];
}

export interface UpdateHostAssignmentRequest {
	role: string;
	/** Ordinal for hook roles; 0 for non-hook roles. Defaults to 0. */
	ordinal?: number;
	plugin_config_id?: string;
	plugin_config?: CreatePluginConfigRequest;
	/** Plugin type for a truly inline assignment with no shared config row (mutually exclusive with plugin_config_id and plugin_config). */
	plugin_type?: string;
	package_identifier?: string;
	config_override?: Record<string, unknown> | null;
	execution_site?: string;
}

export interface TriggerVersionCheckResponse {
	agents_notified: number;
	message: string;
}

export interface UpdateOidcProviderRequest {
	name?: string;
	slug?: string;
	logo_url?: string;
	issuer_url?: string;
	client_id?: string;
	client_secret?: string;
	scopes?: string;
	auto_create_users?: boolean;
	role_claim_path?: string;
	role_mapping?: Record<string, string>;
}

export interface TriggerDiscoveryResponse {
	plugins_queued: number;
	message: string;
}

export interface SoftwareIgnoreResponse {
	id: string;
	name: string;
	host_id?: string | null;
	created_at: string;
}

export interface CreateSoftwareIgnoreRequest {
	name: string;
	host_id?: string | null;
}

export interface UpdateSoftwareItemRequest {
	name?: string;
	featured?: boolean;
	icon_url?: string | null;
}

export type UpdateHistoryStatus = 'queued' | 'pending' | 'in_progress' | 'completed' | 'failed';

export interface UpdateHistoryResponse {
	id: string;
	host_id: string;
	host_name: string;
	software_item_id: string;
	software_item_name: string;
	from_version: string | null;
	to_version: string;
	status: UpdateHistoryStatus;
	actor_type: string;
	actor_id: string;
	started_at: string | null;
	completed_at: string | null;
	output: string | null;
	created_at: string;
	/** Whether the update was dispatched in interactive mode (PTY allocated). */
	interactive: boolean;
	/** Whether any output was dropped because it exceeded the 50 MB output cap. */
	output_truncated: boolean;
}

export interface ReleaseInfoRequest {
	tag: string;
	release_url: string;
}

export interface TriggerUpdateRequest {
	to_version: string;
	release_info?: ReleaseInfoRequest;
}

export interface TriggerUpdateResponse {
	update_history_id: string;
	status: string;
}

export interface ScheduledTaskResponse {
	id: string;
	task_type: string;
	label: string;
	interval_seconds: number;
	jitter_seconds: number;
	enabled: boolean;
	is_running: boolean;
	run_count: number;
	last_run_at: string | null;
	next_run_at: string | null;
	last_error: string | null;
	created_at: string;
	updated_at: string;
}

export interface UpdateScheduledTaskRequest {
	interval_seconds?: number;
	jitter_seconds?: number;
	enabled?: boolean;
}

export interface TriggerScheduledTaskResponse {
	triggered: boolean;
	message: string;
}

export interface ApiTokenResponse {
	id: string;
	name: string;
	revoked_at: string | null;
	created_at: string;
}

export interface ApiTokenListResponse {
	tokens: ApiTokenResponse[];
}

export interface CreateApiTokenRequest {
	name: string;
}

export interface CreateApiTokenResponse {
	id: string;
	token: string;
}

export interface RotateCaResponse {
	message: string;
}

export interface UpdatePluginConfigRequest {
	name?: string;
	config?: Record<string, unknown>;
	enabled?: boolean;
}

// Discovery allowlist

export interface TenantDiscoveryAllowlistEntry {
	id: string;
	plugin_type: string;
	created_at: string;
}

export interface HostDiscoveryAllowlistEntry {
	id: string;
	host_id: string;
	plugin_type: string;
	created_at: string;
}

export interface CreateDiscoveryAllowlistEntryRequest {
	plugin_type: string;
}

// Zeroconf settings

export interface ZeroconfSettingsResponse {
	enabled: boolean;
	url?: string;
	pki_addr?: string;
	ca_fingerprint?: string;
}

export interface UpdateZeroconfSettingsRequest {
	enabled?: boolean;
	url?: string;
	pki_addr?: string;
}

// NATS settings

export interface NatsSettingsResponse {
	url?: string;
	has_url: boolean;
}

export interface UpdateNatsSettingsRequest {
	url?: string | null;
}

// System services

export type SystemServiceStatus = 'pending' | 'approved' | 'rejected' | 'deactivated';

export interface SystemServiceResponse {
	id: string;
	capabilities: string[];
	hostname: string;
	friendly_name: string;
	ip_address: string | null;
	status: SystemServiceStatus;
	client_version: string | null;
	last_seen_at: string | null;
	created_at: string;
	updated_at: string;
	ping_interval_seconds?: number | null;
	cert_lifetime_hours?: number | null;
}

export interface UpdateSystemServiceRequest {
	ping_interval_seconds?: number;
	cert_lifetime_hours?: number;
}

// System enrollment tokens

export interface CreateSystemEnrollmentTokenRequest {
	name: string;
	max_uses?: number;
	expires_in_seconds?: number;
}

export interface SystemEnrollmentTokenCreatedResponse {
	id: string;
	token: string;
	name: string;
	max_uses: number | null;
	current_uses: number;
	expires_at: string | null;
	created_at: string;
	created_by_user_id: string | null;
}

export interface SystemEnrollmentTokenResponse {
	id: string;
	name: string;
	max_uses: number | null;
	current_uses: number;
	expires_at: string | null;
	created_at: string;
	revoked_at: string | null;
	created_by_user_id: string | null;
}

export interface AuditLogEntry {
	id: string;
	actor_id: string;
	actor_type: string;
	auth_method: string;
	http_method: string;
	http_path: string;
	route_pattern: string | null;
	http_status: number;
	client_ip: string | null;
	user_agent: string | null;
	duration_ms: number;
	occurred_at: string;
}

export interface AuditLogListParams {
	page?: number;
	per_page?: number;
	actor_type?: string;
	method?: string;
	status?: number;
	from?: string;
	to?: string;
	actor_id?: string;
}

export type AttestationStatus = 'Verified' | 'NotFound' | 'Unverified';

// ── UI Extensions ─────────────────────────────────────────────────────

export type ExtensionTargeting = 'universal' | 'targeted';

export interface PanelPosition {
	type: 'tab' | 'below' | 'above' | string;
}

export type ExtensionPlacement =
	| { type: 'page'; nav_section: string; icon?: string }
	| { type: 'panel'; target_page: string; position: PanelPosition; tab_group?: string }
	| { type: 'context_menu_group'; target_entity: string; group_label: string }
	| { type: 'table_columns'; target_table: string; columns: ExtensionColumn[] };

export interface ExtensionColumn {
	key: string;
	label: string;
	data_action: string;
}

export interface TableColumn {
	key: string;
	label: string;
	sortable?: boolean;
}

export type FieldType =
	| 'text'
	| 'password'
	| 'number'
	| 'select'
	| 'multi_select'
	| 'textarea'
	| 'toggle'
	| 'hidden'
	| 'ssh_private_key'
	| string;

export interface SelectOption {
	value: string;
	label: string;
}

/** Dynamic data source for a `Select` field, loaded at form-open time. */
export type SelectSource =
	| {
			type: 'rest_api';
			/** API path relative to the controller base URL (e.g., `"/api/v1/hosts"`). */
			path: string;
			/** Field in each response item to use as the submitted option value. */
			value_field: string;
			/** Field in each response item to use as the human-readable label. */
			label_field: string;
	  }
	| {
			type: 'action';
			/** Extension action ID to invoke. Must return `{ options: [{ value, label }] }`. */
			action_id: string;
	  };

/** Condition for conditional field visibility. */
export interface VisibleWhen {
	/** Key of the controlling field. */
	field: string;
	/** Field is visible when the controlling field's value is in this list. */
	values: string[];
}

export interface FieldDef {
	key: string;
	label: string;
	field_type: FieldType;
	required: boolean;
	placeholder?: string;
	help_text?: string;
	default_value?: string;
	options?: SelectOption[];
	/** When set, options are loaded dynamically from the given source. Takes precedence over `options`. */
	select_source?: SelectSource;
	/** When true, the field value is encrypted client-side before being sent to the service. */
	sensitive?: boolean;
	/** When true, the textarea value is a newline-separated list serialized as a JSON string array. */
	list?: boolean;
	/** When set, the field is only visible when the controlling field's value matches. */
	visible_when?: VisibleWhen;
}

export interface FormDef {
	fields: FieldDef[];
	/** Action ID to invoke when the form opens, to pre-populate field values from the response. */
	pre_load_action?: string;
	/** Action IDs rendered as buttons below the form save button. */
	footer_actions?: string[];
}

export interface WizardStep {
	step_id: string;
	label: string;
	form: FormDef;
	submit_action?: string;
	/** When true, render the previous step's response data instead of a form. */
	render_previous_response?: boolean;
}

export type ActionUi =
	| { type: 'form'; fields: FieldDef[]; pre_load_action?: string }
	| { type: 'wizard'; steps: WizardStep[] };

/** Describes a direct REST API call as the submit target for an action form. */
export interface ApiSubmitDef {
	/** HTTP method, e.g. `"POST"`. */
	method: string;
	/** API path relative to the base URL, e.g. `"/api/v1/plugin-configs"`. */
	path: string;
	/** JSON body template — string leaves matching `{{field_name}}` or `{{field_name:coercion}}` are substituted. */
	body: Record<string, unknown>;
	/** Field in the JSON response containing the new item's ID (used for auto-selection). */
	response_id_field?: string;
	/** Field in the JSON response containing the new item's display label. */
	response_label_field?: string;
}

export interface RowVisibleWhen {
	field: string;
	condition: 'present' | 'absent';
}

export interface ActionDef {
	action_id: string;
	label: string;
	ui?: ActionUi;
	permission?: string;
	destructive: boolean;
	timeout_seconds?: number;
	/** When set, form submission calls this REST API endpoint directly instead of routing through the extension proxy. */
	api_submit?: ApiSubmitDef;
	/** Conditional visibility for row actions: show only when the condition on a row data field is met. */
	row_visible_when?: RowVisibleWhen;
	/** Row data field to use as the entity name in the confirmation dialog for destructive actions. */
	confirm_entity_field?: string;
	/** When true, this action supports batch execution with multiple selected rows. */
	batch_action?: boolean;
}

export type ContextSelectorSource =
	| { type: 'action'; action_id: string }
	| { type: 'plugin_configs'; plugin_type: string };

export interface ContextSelectorDef {
	param_key: string;
	label: string;
	source: ContextSelectorSource;
	/** When set, a "Add" button appears next to the selector. References an action_id from the action library. */
	add_action?: string;
	/** Message shown when no options exist and no add_action is set. */
	empty_message?: string;
}

export type ExtensionUi =
	| {
			type: 'data_table';
			columns: TableColumn[];
			data_action: string;
			/** Action ID references (resolved via the action library). */
			row_actions: string[];
			/** Action ID references (resolved via the action library). */
			primary_actions: string[];
			context_selector?: ContextSelectorDef;
			/** Default number of items per page. When absent, defaults to 20. */
			default_per_page?: number;
	  }
	| { type: 'form'; fields: FieldDef[]; pre_load_action?: string; footer_actions?: string[] }
	| { type: 'key_value'; data_action: string }
	| { type: 'actions'; actions: string[] };

export interface ExtensionManifest {
	id: string;
	label: string;
	priority: number;
	placement: ExtensionPlacement;
	required_permission?: string;
	targeting: ExtensionTargeting;
	ui: ExtensionUi;
}

export interface ExtensionResponse {
	id: string;
	label: string;
	priority: number;
	placement: ExtensionPlacement;
	required_permission?: string;
	targeting: ExtensionTargeting;
	ui: ExtensionUi;
	/** Resolved action catalogue for this extension's source. */
	actions: ActionDef[];
	provider_count: number;
}

// ── Batch Actions ─────────────────────────────────────────────────────

export interface BatchActionRequest {
	action: string;
	ids: string[];
}

export interface BatchActionSuccess {
	id: string;
}

export interface BatchActionFailure {
	id: string;
	error: string;
}

export interface BatchActionResponse {
	succeeded: BatchActionSuccess[];
	failed: BatchActionFailure[];
}

export interface ExtensionProviderInfo {
	service_id: string;
	service_label: string;
	hostname: string | null;
	/** Base64-encoded uncompressed P-256 public key (65 bytes) used for ECIES sealed-box encryption. */
	encryption_public_key?: string;
}

// ── Notification Rules + Log ──

export type NotificationEventType =
	| 'update_available'
	| 'update_completed'
	| 'update_failed'
	| 'new_software_discovered'
	| 'new_service_enrolled'
	| 'ca_rotated'
	| 'batch_update_completed'
	| 'batch_update_partially_completed'
	| 'stdin_attention';

export type NotificationDeliveryStatus = 'pending' | 'delivered' | 'failed';

export interface NotificationChannelSummary {
	id: string;
	name: string;
	channel_type: string;
}

export interface NotificationRuleResponse {
	id: string;
	channel_id: string;
	event_type: string;
	host_id: string | null;
	software_item_id: string | null;
	plugin_type: string | null;
	enabled: boolean;
	created_at: string;
}

export interface NotificationLogEntry {
	id: string;
	channel_id: string;
	rule_id: string;
	event_type: string;
	status: string;
	error_message: string | null;
	created_at: string;
	delivered_at: string | null;
}

// ── Reset Data ────────────────────────────────────────────────────────

export interface ResetDataRequest {
	confirm: string;
}

export interface ResetDeletedCounts {
	hosts: number;
	software_items: number;
	plugin_configs: number;
	host_tags: number;
	update_history: number;
	update_batches: number;
}

export interface ResetDataResponse {
	deleted: ResetDeletedCounts;
}
