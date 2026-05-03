mod proxy;
mod registry;

pub use proxy::entity_enrichment;
pub use proxy::{
    AppStateSurfaceActionController, PluginSurfaceActionInvoker, PluginSurfaceLocalExecutor,
    SurfaceCallerOrigin, SurfaceInvokeRequest, SurfaceLocalActionExecutor, SurfaceProxy,
    SurfaceProxyError, map_surface_action_error,
};
pub use registry::{
    ResolvedSurfaceAction, ResolvedSurfaceRead, SurfaceCatalogItem, SurfaceProviderRejection,
    SurfaceProviderRejectionCode, SurfaceProviderRejectionReason, SurfaceProviderSummary,
    SurfaceRegistry, SurfaceRegistryConfig, SurfaceRegistryError, SurfaceRegistryLookupError,
};
