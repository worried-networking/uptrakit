// Surface API functions — hand-written (surfaces have 0 utoipa paths) but routed
// through the configured client so auth/refresh/ApiError plumbing is shared.
import { apiClient, BASE_PATH } from './client';
import type {
	InvokeSurfaceInteractionRequest,
	SurfaceProviderInfo,
	SurfaceReadResponse,
	SurfaceResponse
} from '../surfaces/contract';

export async function listSurfaces(options?: { slot?: string; page?: string }): Promise<SurfaceResponse[]> {
	const { data } = await apiClient.get({
		url: `${BASE_PATH}/surfaces`,
		query: { slot: options?.slot, page: options?.page }
	});
	return data as SurfaceResponse[];
}

export async function listSurfaceProviders(surfaceId: string): Promise<SurfaceProviderInfo[]> {
	const { data } = await apiClient.get({
		url: `${BASE_PATH}/surfaces/${encodeURIComponent(surfaceId)}/providers`
	});
	return data as SurfaceProviderInfo[];
}

export async function getSurfaceRead(surfaceId: string): Promise<SurfaceReadResponse> {
	const { data } = await apiClient.get({
		url: `${BASE_PATH}/surfaces/${encodeURIComponent(surfaceId)}/read`
	});
	return data as SurfaceReadResponse;
}

export async function invokeSurfaceInteraction(
	surfaceId: string,
	interactionId: string,
	body: InvokeSurfaceInteractionRequest
): Promise<unknown> {
	const { data } = await apiClient.post({
		url: `${BASE_PATH}/surfaces/${encodeURIComponent(surfaceId)}/interactions/${encodeURIComponent(interactionId)}`,
		body
	});
	return data;
}
