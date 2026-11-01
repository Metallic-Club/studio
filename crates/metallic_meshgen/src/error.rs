//! Errors returned when a mesh request is invalid.

/// Why a [`crate::MeshDesc`] could not be turned into a mesh.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum MeshGenError {
    /// A size component is not finite or not positive.
    #[error("size must be finite and positive on every axis, got {0:?}")]
    InvalidSize([f32; 3]),
    /// A size component is larger than [`crate::MAX_SIZE`].
    #[error("size {size:?} exceeds the maximum of {max} studs per axis")]
    SizeTooLarge {
        /// The requested size.
        size: [f32; 3],
        /// The largest allowed component.
        max: f32,
    },
    /// A numeric parameter is negative, NaN or infinite.
    #[error("parameter `{name}` must be finite and not negative, got {value}")]
    InvalidParameter {
        /// Name of the field, for example `bevel.width`.
        name: &'static str,
        /// The rejected value.
        value: f32,
    },
    /// [`crate::Shape::from_name`] was given a name it does not know.
    #[error("unknown shape name `{0}`")]
    UnknownShape(String),
}
