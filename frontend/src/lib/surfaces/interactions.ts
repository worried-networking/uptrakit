import { sealedBoxEncrypt } from '$lib/api';
import type { InteractionDescriptor, InvokeSurfaceInteractionRequest, SurfaceResponse } from '$lib/surfaces/contract';

export interface SurfaceEncryptionContext {
	keyId: string;
	algorithm: 'ecies_p256';
	publicKey: string;
}

export type EncryptSensitivePayload = (plaintext: string, publicKey: string) => Promise<string>;

export function clampSurfaceTabIndex(index: number, tabsCount: number): number {
	if (tabsCount <= 0) {
		return 0;
	}
	if (index < 0) {
		return 0;
	}
	if (index >= tabsCount) {
		return tabsCount - 1;
	}
	return index;
}

export function getSurfaceDescriptorRenderKey(surface: SurfaceResponse): string {
	return JSON.stringify({
		surface_id: surface.surface_id,
		label: surface.label,
		priority: surface.priority,
		slot: surface.slot,
		scope: surface.scope,
		targeting: surface.targeting,
		required_permission: surface.required_permission ?? null,
		provider_kind: surface.provider_kind,
		required_capabilities: surface.required_capabilities,
		root_node: surface.root_node
	});
}

export async function buildSurfaceInteractionRequest(
	interaction: InteractionDescriptor,
	params: Record<string, unknown>,
	options?: {
		targetProviderId?: string;
		encryption?: SurfaceEncryptionContext;
		encryptSensitivePayload?: EncryptSensitivePayload;
	}
): Promise<InvokeSurfaceInteractionRequest> {
	const sensitiveFields = interaction.sensitive_fields ?? [];
	const sensitiveFieldSet = new Set(sensitiveFields);
	const regularParams: Record<string, unknown> = {};
	const sensitiveParams: Record<string, unknown> = {};

	for (const [key, value] of Object.entries(params)) {
		if (sensitiveFieldSet.has(key)) {
			sensitiveParams[key] = value;
		} else {
			regularParams[key] = value;
		}
	}

	const request: InvokeSurfaceInteractionRequest = {
		params: regularParams,
		target_provider_id: options?.targetProviderId,
		timeout_seconds: interaction.timeout_seconds
	};

	if (sensitiveFields.length === 0) {
		return request;
	}

	if (Object.keys(sensitiveParams).length === 0) {
		return request;
	}

	const requiresEncryptedSensitiveEnvelope = interaction.transport.mode === 'provider_proxied';
	if (!requiresEncryptedSensitiveEnvelope) {
		request.params = { ...regularParams, ...sensitiveParams };
		return request;
	}

	const encryption = options?.encryption;
	if (!encryption) {
		throw new Error(
			`Interaction "${interaction.interaction_id}" declares sensitive_fields but no encryption metadata is available.`
		);
	}

	const encrypt = options?.encryptSensitivePayload ?? sealedBoxEncrypt;
	const ciphertext = await encrypt(JSON.stringify(sensitiveParams), encryption.publicKey);
	request.encrypted_sensitive_params = {
		key_id: encryption.keyId,
		algorithm: encryption.algorithm,
		ciphertext_b64: ciphertext
	};

	return request;
}
