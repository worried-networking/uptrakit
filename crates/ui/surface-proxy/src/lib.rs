mod proxy;
mod registry;

pub use proxy::entity_enrichment;
pub use proxy::{
    AppStateSurfaceActionController, PluginOpsSurfaceActionInvoker, PluginSurfaceActionInvoker,
    PluginSurfaceLocalExecutor, SurfaceCallerOrigin, SurfaceInvokeRequest,
    SurfaceLocalActionExecutor, SurfaceProxy, SurfaceProxyError,
};
pub use registry::{
    ResolvedSurfaceAction, ResolvedSurfaceRead, SurfaceCatalogItem, SurfaceProviderRejection,
    SurfaceProviderRejectionCode, SurfaceProviderRejectionReason, SurfaceProviderSummary,
    SurfaceRegistry, SurfaceRegistryConfig, SurfaceRegistryError, SurfaceRegistryLookupError,
};
