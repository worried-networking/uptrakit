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

export type ServiceType = 'agent' | 'mqtt' | 'ssh_agent';

export type ServiceStatus = 'pending' | 'approved' | 'rejected' | 'deactivated';

export interface ServiceResponse {
	id: string;
	service_type: ServiceType;
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

export interface EnrollmentTokenStatusesResponse {
	agent: EnrollmentTokenStatus;
	mqtt: EnrollmentTokenStatus;
	ssh_agent: EnrollmentTokenStatus;
}

export interface CombinedSettingsResponse {
	registration: RegistrationSettings;
	authentication: AuthenticationSettings;
	agent_certificates: AgentCertificateSettings;
	enrollment_tokens: EnrollmentTokenStatusesResponse;
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
	provider_config_id?: string;
	provider_config?: CreateProviderConfigRequest;
	package_identifier?: string;
	config_override?: Record<string, unknown> | null;
	enabled?: boolean;
}

export interface SoftwareItemResponse {
	id: string;
	name: string;
	provider_config_id: string;
	provider_config_name: string;
	provider_type: string;
	package_identifier: string;
	config_override: Record<string, unknown> | null;
	enabled: boolean;
	last_checked_at: string | null;
	host_count: number;
	created_at: string;
	updated_at: string;
}

export interface SoftwareItemHostSummary {
	host_id: string;
	hostname: string;
	friendly_name: string;
	installed_version: string | null;
	installed_version_detected_at: string | null;
	last_updated_at: string | null;
	linked_at: string;
}

export interface SoftwareItemDetailResponse extends SoftwareItemResponse {
	hosts: SoftwareItemHostSummary[];
}

export interface AssignHostsRequest {
	host_ids: string[];
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
