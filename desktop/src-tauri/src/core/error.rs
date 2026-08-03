use thiserror::Error;

pub type CoreResult<T> = Result<T, CoreError>;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("filesystem operation failed")]
    Filesystem,
    #[error("database operation failed")]
    Database,
    #[error("selected root does not contain exactly two campus directories")]
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
    #[error("derived output path is unsafe")]
    UnsafeDerivedPath,
    #[error("a representative asset with saved regions cannot be deselected")]
    RepresentativeHasRegions,
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
