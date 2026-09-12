//! Shadow-awareness policy for standard-library rewrites.
//!
//! Legacy passes decide whether a standard library name (such as `math` or
//! `tostring`) is shadowed by scanning the block's identifier names. Passes
//! that have a [`SemanticModel`] can instead use
//! [`SemanticModel::name_is_transparent_global`], which recognizes read-only
//! aliases (`local math = math`) and returns the same answer as the name scan
//! whenever a local can hold a different value.

use crate::nodes::Block;
use crate::semantic::SemanticModel;

use super::tracker::IdentifierTracker;

pub(crate) enum ShadowPolicy<'a> {
    /// Name-based tracking: the name is shadowed when it is declared anywhere
    /// in the block.
    Names(IdentifierTracker),
    /// Model-based tracking: the name is transparent when every declaration of
    /// it is a read-only global alias.
    Model(&'a SemanticModel),
}

impl<'a> ShadowPolicy<'a> {
    /// Builds the legacy name-based policy for a block.
    pub(crate) fn from_block(block: &mut Block) -> Self {
        Self::Names(IdentifierTracker::collect_block(block))
    }

    /// Builds a model-based policy.
    pub(crate) fn from_model(model: &'a SemanticModel) -> Self {
        Self::Model(model)
    }

    /// Returns whether references to `name` can be treated as references to the
    /// global symbol of that name.
    pub(crate) fn is_transparent(&self, name: &str) -> bool {
        match self {
            ShadowPolicy::Names(tracker) => !tracker.is_declared(name),
            ShadowPolicy::Model(model) => model.name_is_transparent_global(name),
        }
    }
}
