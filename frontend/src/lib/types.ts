export interface ErrorResponse {
	error: string;
	code?: string;
}

export enum Permission {
	ViewSettings = 'view_settings',
	ManageSettings = 'manage_settings',
	ViewAgents = 'view_agents',
	ManageAgents = 'manage_agents',
	ManageGlobalSettings = 'manage_global_settings',
	ViewSoftware = 'view_software',
	ManageSoftware = 'manage_software',
	ViewHosts = 'view_hosts',
	ManageHosts = 'manage_hosts'
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
	renewal_window_hours: number;
}

export interface UpdateAgentCertificateSettings {
	lifetime_days?: number;
	renewal_window_hours?: number;
}

export interface EnrollmentTokenStatus {
	configured: boolean;
}

export interface CombinedSettingsResponse {
	registration: RegistrationSettings;
	authentication: AuthenticationSettings;
	agent_certificates: AgentCertificateSettings;
	enrollment_token: EnrollmentTokenStatus;
}

export interface EnrollmentTokenResponse {
	token: string;
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
	extra_sans: string[];
	https_addr: string;
}

export interface UpdateNetworkSettings {
	trusted_proxies?: string[];
	real_ip_header?: string;
	extra_sans?: string[];
	https_addr?: string;
}

export type MqttTransport = 'tcp' | 'tls';
export type MqttConnectionStatus = 'online' | 'offline' | 'connecting';

export interface MqttClientResponse {
	id: string;
	enabled: boolean;
	transport: MqttTransport;
	host: string;
	port: number;
	url: string;
	client_id: string;
	username: string | null;
	has_password: boolean;
	has_ca_cert: boolean;
	topic_prefix: string;
	connection_status: MqttConnectionStatus;
}

export interface CreateMqttClient {
	url?: string;
	transport?: MqttTransport;
	host?: string;
	port?: number;
	enabled?: boolean;
	client_id?: string;
	username?: string;
	password?: string;
	ca_pem?: string;
	topic_prefix?: string;
}

export interface UpdateMqttClient {
	url?: string;
	transport?: MqttTransport;
	host?: string;
	port?: number;
	enabled?: boolean;
	client_id?: string;
	username?: string | null;
	password?: string;
	ca_pem?: string | null;
	topic_prefix?: string;
}

export interface MqttLimitResponse {
	max_clients_per_tenant: number;
}

export interface UpdateMqttLimitRequest {
	max_clients_per_tenant: number;
}

export interface HostAgentSummary {
	id: string;
	friendly_name: string;
	status: ServiceStatus;
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

export interface ProviderConfigResponse {
	id: string;
	name: string;
	provider_type: string;
	config: Record<string, unknown>;
	enabled: boolean;
	created_at: string;
	updated_at: string;
}

export interface CreateProviderConfigRequest {
	name: string;
	provider_type: string;
	config: Record<string, unknown>;
	enabled?: boolean;
}

export interface CreateSoftwareItemRequest {
	name: string;
	enabled?: boolean;
}

export interface SoftwareItemResponse {
	id: string;
	name: string;
	provider_types: string[];
	enabled: boolean;
	discovery_state?: 'pending' | 'approved' | null;
	last_checked_at: string | null;
	host_count: number;
	latest_version?: string | null;
	update_available: boolean;
	created_at: string;
	updated_at: string;
}

export interface SoftwareItemHostSummary {
	host_id: string;
	hostname: string;
	friendly_name: string;
	provider_config_id: string;
	provider_config_name: string;
	provider_type: string;
	package_identifier: string;
	config_override: Record<string, unknown> | null;
	installed_version: string | null;
	installed_version_detected_at: string | null;
	last_updated_at: string | null;
	linked_at: string;
	latest_version?: string | null;
	update_available: boolean;
}

export interface SoftwareItemDetailResponse extends SoftwareItemResponse {
	hosts: SoftwareItemHostSummary[];
}

export interface HostSoftwareAssignment {
	host_id: string;
	provider_config_id?: string;
	provider_config?: CreateProviderConfigRequest;
	package_identifier?: string;
	config_override?: Record<string, unknown> | null;
}

export interface AssignHostsRequest {
	host_assignments: HostSoftwareAssignment[];
}

export interface UpdateHostAssignmentRequest {
	provider_config_id?: string;
	provider_config?: CreateProviderConfigRequest;
	package_identifier?: string;
	config_override?: Record<string, unknown> | null;
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
	providers_queued: number;
	message: string;
}

export interface DiscardDiscoveredResponse {
	discarded_count: number;
}

export interface AutodiscoveryIgnoreResponse {
	id: string;
	provider_config_id: string;
	provider_config_name: string;
	provider_type: string;
	package_identifier: string;
	created_at: string;
}

export interface CreateAutodiscoveryIgnoreRequest {
	provider_config_id: string;
	package_identifier: string;
}

export interface UpdateSoftwareItemRequest {
	name?: string;
	enabled?: boolean;
}

export type UpdateHistoryStatus = 'pending' | 'in_progress' | 'completed' | 'failed';

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
	cron_expression: string;
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
	cron_expression?: string;
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

export interface UpdateProviderConfigRequest {
	name?: string;
	config?: Record<string, unknown>;
	enabled?: boolean;
}
