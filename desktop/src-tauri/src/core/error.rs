use thiserror::Error;

pub type CoreResult<T> = Result<T, CoreError>;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("filesystem operation failed")]
    Filesystem,
    #[error("database operation failed")]
    Database,
    #[error("database schema version is unsupported")]
    DatabaseSchemaUnsupported,
    #[error("selected root must contain at least one valid campus directory")]
    InvalidCampusLayout,
    #[error("source root and project work root must not overlap")]
    SourceWorkOverlap,
    #[error("no active scan session")]
    NoActiveSession,
    #[error("requested item is not in the current scan session")]
    UnknownSessionItem,
    #[error("only image assets can be representative")]
    NotImageAsset,
    #[error("source image is unsafe to open")]
    UnsafeSourceAsset,
    #[error("the asset is not currently selected as a representative asset")]
    RepresentativeNotSelected,
    #[error("the asset is not a representative asset")]
    AssetNotRepresentative,
    #[error("template is not loaded in the current session")]
    TemplateNotLoaded,
    #[error("template validation failed")]
    InvalidTemplate,
    #[error("the fixed project template is missing")]
    FixedTemplateMissing,
    #[error("the fixed project template does not match its recorded checksum")]
    FixedTemplateChecksumMismatch,
    #[error("a different fixed project template is already installed")]
    FixedTemplateAlreadyInstalled,
    #[error("the replacement template reuses an installed template ID and version with different content; increase the version or use a new template ID")]
    TemplateReplacementIdentityConflict,
    #[error("new regions must use the current fixed project template")]
    TemplateNotActive,
    #[error("label values do not satisfy the template")]
    InvalidLabels,
    #[error("polygon is invalid")]
    InvalidPolygon,
    #[error("an immutable template snapshot conflicts with this version")]
    TemplateSnapshotConflict,
    #[error("invalid opaque identifier")]
    InvalidOpaqueId,
    #[error("relative work-directory path is invalid")]
    InvalidWorkPath,
    #[error("the installation-adjacent project storage is unavailable")]
    WorkRootUnavailable,
    #[error("derived output path is unsafe")]
    UnsafeDerivedPath,
    #[error("a representative asset with saved regions cannot be deselected")]
    RepresentativeHasRegions,
    #[error("requested region does not exist")]
    RegionNotFound,
    #[error("requested region is no longer active")]
    RegionNotActive,
    #[error("region geometry changed before this refinement could be saved")]
    RegionGeometryConflict,
    #[error("requested asset is not a source video")]
    NotVideoAsset,
    #[error("captured frame data is invalid")]
    InvalidCapture,
    #[error("image dimensions are invalid")]
    InvalidAssetDimensions,
    #[error("the saved data root is unavailable")]
    SavedRootUnavailable,
    #[error("credential store is unavailable")]
    CredentialStoreUnavailable,
    #[error("secret material is invalid")]
    InvalidSecret,
    #[error("internal state is unavailable")]
    StateUnavailable,
    #[error("test fault injected")]
    FaultInjected,
}

impl From<std::io::Error> for CoreError {
    fn from(_: std::io::Error) -> Self {
        Self::Filesystem
    }
}

impl From<rusqlite::Error> for CoreError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Database
    }
}

impl From<serde_json::Error> for CoreError {
    fn from(_: serde_json::Error) -> Self {
        Self::InvalidTemplate
    }
}
