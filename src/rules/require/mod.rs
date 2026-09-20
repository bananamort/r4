//! Model-first `require` conversion, path aliases and module bundling.
//!
//! Dependency discovery and path resolution live in [`crate::modules`]; the
//! semantic connection between modules lives in
//! [`crate::semantic::MultiModuleModel`]. This module is the transformation
//! layer: it converts resolved `require` calls, applies alias mappings and
//! bundles a resolved graph.

mod alias;
mod bundler;
mod convert;
mod link;

pub use alias::{apply_path_aliases, AliasError, AliasRequireConverter};
pub use bundler::{bundle_modules, BundleError};
pub use convert::{convert_require, ConvertRequireError, RequirePathConverter};

pub use crate::modules::{
    classify_require_call, MemoryModuleProvider, ModuleLoadError, ModuleProvider, RequireCall,
    REQUIRE_NAME,
};
