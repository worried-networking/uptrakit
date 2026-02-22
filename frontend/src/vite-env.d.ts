/// <reference types="vite/client" />

interface ImportMetaEnv {
	/** API base path. Defaults to `/api/v1`. Set at build time when the SPA is
	 * served from a non-root path (e.g. behind a reverse proxy with a
	 * sub-path prefix). See `docs/end-user/deployment/reverse-proxy.md`. */
	readonly VITE_API_BASE?: string;
}

interface ImportMeta {
	readonly env: ImportMetaEnv;
}
