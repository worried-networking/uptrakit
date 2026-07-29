mod proxy;
mod registry;

pub use proxy::entity_enrichment;
pub use proxy::{
    AppStateSurfaceActionController, CONTROLLER_LOCAL_EXECUTOR_TABLE, ExecutorTier,
    PluginSurfaceActionInvoker, PluginSurfaceLocalExecutor, SurfaceCallerOrigin,
    SurfaceInvokeRequest, SurfaceInvokerContext, SurfaceLocalActionExecutor, SurfaceProxy,
    SurfaceProxyError, map_surface_action_error,
};
pub use registry::{
    DenyAllPluginProviders, ResolvedSurfaceAction, ResolvedSurfaceRead, SurfaceCatalogItem,
    SurfaceProviderRejection, SurfaceProviderRejectionCode, SurfaceProviderRejectionReason,
    SurfaceProviderSummary, SurfaceProviderVisibility, SurfaceRegistry, SurfaceRegistryConfig,
    SurfaceRegistryError, SurfaceRegistryLookupError,
};

#[cfg(any(test, feature = "testing"))]
pub use registry::AllProvidersVisible;
