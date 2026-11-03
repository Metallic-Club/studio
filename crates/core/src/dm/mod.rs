//! The data model: the instance tree and every property value.

/// The tree of instances that makes up a game (`game` in scripts), and the only source of truth for their
/// properties.
///
/// This is a placeholder so that the descriptor function types (methods, computed properties, class behaviour)
/// can name it. #14 fills it in: instance storage, the single write path, dirty bits and signals.
#[derive(Debug)]
pub struct DataModel {
    _private: (),
}
