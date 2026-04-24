/**
 * Known entity types for surface entity links.
 *
 * `string & {}` keeps autocomplete for known values while accepting unknown
 * types from newer backend versions (forward-compatible).
 */
export type SurfaceEntityType = 'host' | (string & {});

/**
 * Returns the frontend route for a given entity type and ID, or `null` if
 * the entity type has no known route in this frontend version.
 *
 * The `default` arm is always required — future entity types must not cause
 * a TypeScript exhaustiveness error here.
 */
export function entityRoute(entityType: SurfaceEntityType, entityId: string): string | null {
	switch (entityType) {
		case 'host':
			return `/hosts/${entityId}`;
		default:
			return null;
	}
}
