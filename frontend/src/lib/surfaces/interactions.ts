import {
	deleteSurfaceInteraction,
	deleteSurfaceInteractionItem,
	invokeSurfaceInteraction,
	readSurfaceInteraction,
	readSurfaceInteractionItem,
	sealedBoxEncrypt,
	updateSurfaceInteraction,
	updateSurfaceInteractionItem
} from '$lib/api';
import type {
	InvokeSurfaceInteractionRequest as GeneratedInvokeRequest,
	ReadSurfaceInteractionData,
	ReadSurfaceInteractionItemData
} from '$lib/api';
import type {
	ActionRef,
	InteractionDescriptor,
	InteractionHttpMethod,
	InvokeSurfaceInteractionRequest,
	SurfaceResponse
} from '$lib/surfaces/contract';

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
		required_action: surface.required_action ?? null,
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

export function resolveInteraction(
	interactions: InteractionDescriptor[],
	interactionId: string,
	httpMethod?: InteractionHttpMethod
): InteractionDescriptor | undefined {
	const candidates = interactions.filter((interaction) => interaction.interaction_id === interactionId);
	if (httpMethod !== undefined) {
		return candidates.find((interaction) => interaction.http_method === httpMethod);
	}
	if (candidates.length === 1) {
		return candidates[0];
	}
	return undefined;
}

export function actionRefId(ref: ActionRef): string {
	return typeof ref === 'string' ? ref : ref.interaction_id;
}

export function actionRefMethod(ref: ActionRef): InteractionHttpMethod | undefined {
	return typeof ref === 'string' ? undefined : ref.http_method;
}

export async function dispatchSurfaceInteraction(
	surfaceId: string,
	interaction: InteractionDescriptor,
	request: InvokeSurfaceInteractionRequest,
	options?: { itemId?: string }
): Promise<unknown> {
	const path = {
		surface_id: surfaceId,
		interaction_id: interaction.interaction_id
	};
	const itemPath = options?.itemId ? { ...path, item_id: options.itemId } : undefined;
	switch (interaction.http_method) {
		case 'get': {
			const query = {
				target_provider_id: request.target_provider_id ?? undefined,
				timeout_seconds: request.timeout_seconds ?? undefined,
				...Object.fromEntries(
					Object.entries((request.params as Record<string, unknown>) ?? {})
						// Drop unset params instead of sending literal "null"/"undefined"
						// strings — matches the old POST-body omission semantics.
						.filter(([, value]) => value != null)
						.map(([key, value]) => [key, String(value)])
				)
			};
			// The generated query type is closed (reserved keys only); dynamic
			// DataLoad params are string-passthrough by contract, so widen here.
			// Sanctioned escape hatch — see frontend/AGENTS.md.
			if (itemPath) {
				const { data } = await readSurfaceInteractionItem({
					path: itemPath,
					query: query as ReadSurfaceInteractionItemData['query']
				});
				return data;
			}
			const { data } = await readSurfaceInteraction({
				path,
				query: query as ReadSurfaceInteractionData['query']
			});
			return data;
		}
		case 'put': {
			const call = itemPath
				? updateSurfaceInteractionItem({ path: itemPath, body: request as unknown as GeneratedInvokeRequest })
				: updateSurfaceInteraction({ path, body: request as unknown as GeneratedInvokeRequest });
			const { data } = await call;
			return data;
		}
		case 'delete': {
			const call = itemPath
				? deleteSurfaceInteractionItem({ path: itemPath, body: request as unknown as GeneratedInvokeRequest })
				: deleteSurfaceInteraction({ path, body: request as unknown as GeneratedInvokeRequest });
			const { data } = await call;
			return data;
		}
		default: {
			const { data } = await invokeSurfaceInteraction({
				path,
				body: request as unknown as GeneratedInvokeRequest
			});
			return data;
		}
	}
}
