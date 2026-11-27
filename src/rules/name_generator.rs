//! Strict, model-first fresh identifier allocation.

use std::collections::HashSet;
use std::fmt;

use crate::process::utils::{identifier_permutator, is_valid_identifier, CharPermutator};
use crate::semantic::SemanticModel;

use super::rename_variables::{DEFAULT_GLOBALS, ROBLOX_GLOBALS};

/// An error produced by [`NameGenerator::generate_with_prefix`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NameGeneratorError {
    prefix: String,
}

impl NameGeneratorError {
    /// Returns the prefix that cannot form an identifier.
    #[inline]
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// Returns the stable code of this error.
    pub fn code(&self) -> &'static str {
        "name-generator/invalid-prefix"
    }
}

impl fmt::Display for NameGeneratorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "prefix `{}` cannot begin a valid Lua identifier",
            self.prefix
        )
    }
}

impl std::error::Error for NameGeneratorError {}

/// Allocates identifiers that are guaranteed to be unused by the analyzed
/// program.
///
/// The generator reserves, by construction:
///
/// * every symbol name in the [`SemanticModel`], which is a superset of the
///   identifiers visible in any single scope;
/// * the default Lua/Luau and Roblox host globals;
/// * every identifier it has already generated.
///
/// Consequently a generated name cannot capture or be captured by any existing
/// binding, regardless of scope.
#[derive(Debug)]
pub struct NameGenerator {
    reserved: HashSet<String>,
    permutator: CharPermutator,
}

impl NameGenerator {
    /// Builds a generator reserved against every model symbol, type symbol and
    /// host global.
    pub fn new(model: &SemanticModel) -> Self {
        let mut reserved: HashSet<String> = model
            .symbols
            .iter()
            .map(|symbol| symbol.name.clone())
            .collect();
        reserved.extend(model.types.iter().map(|symbol| symbol.name.clone()));
        reserved.extend(model.labels.iter().map(|label| label.name.clone()));
        reserved.extend(DEFAULT_GLOBALS.iter().map(|name| (*name).to_owned()));
        reserved.extend(ROBLOX_GLOBALS.iter().map(|name| (*name).to_owned()));

        Self {
            reserved,
            permutator: identifier_permutator(),
        }
    }

    /// Returns whether a name is already reserved.
    pub fn is_reserved(&self, name: &str) -> bool {
        self.reserved.contains(name)
    }

    /// Reserves an additional name so it can never be generated.
    ///
    /// Bundlers use this to reserve the symbols of every module they inline,
    /// not just the symbols of the root model.
    pub fn reserve(&mut self, name: impl Into<String>) {
        self.reserved.insert(name.into());
    }

    /// Returns a fresh short identifier.
    pub fn generate(&mut self) -> String {
        loop {
            let candidate = self
                .permutator
                .next()
                .expect("the identifier permutator always yields names");
            if is_valid_identifier(&candidate) && !self.reserved.contains(&candidate) {
                self.reserved.insert(candidate.clone());
                return candidate;
            }
        }
    }

    /// Returns a fresh identifier beginning with `prefix`.
    ///
    /// When the prefix itself is available it is returned unchanged; otherwise
    /// the first free `prefix{N}` is returned. The allocated name is always
    /// registered, so repeated calls never return the same identifier.
    ///
    /// # Errors
    ///
    /// Returns a [`NameGeneratorError`] when `prefix` cannot begin a valid
    /// identifier (a leading digit, a space, ...): no `prefix{N}` could be valid
    /// either. The generator never substitutes an unrelated name.
    pub fn generate_with_prefix(
        &mut self,
        prefix: impl Into<String>,
    ) -> Result<String, NameGeneratorError> {
        let prefix = prefix.into();

        if !is_valid_identifier(&format!("{prefix}0")) {
            return Err(NameGeneratorError { prefix });
        }

        if is_valid_identifier(&prefix) && !self.reserved.contains(&prefix) {
            self.reserved.insert(prefix.clone());
            return Ok(prefix);
        }

        let mut counter = 1_u32;
        loop {
            let candidate = format!("{prefix}{counter}");
            if is_valid_identifier(&candidate) && !self.reserved.contains(&candidate) {
                self.reserved.insert(candidate.clone());
                return Ok(candidate);
            }
            counter += 1;
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::rules::rename_variables::{DEFAULT_GLOBALS, ROBLOX_GLOBALS};
    use crate::rules::test_support::analyze_with_tokens;

    #[test]
    fn avoids_model_symbols_and_host_globals() {
        let (_, model) =
            analyze_with_tokens("local a = 1\nlocal __R4_SELECT = 2\nreturn a + __R4_SELECT");

        let mut generator = NameGenerator::new(&model);
        let names: Vec<String> = (0..5).map(|_| generator.generate()).collect();

        let unique: std::collections::HashSet<&String> = names.iter().collect();
        assert_eq!(unique.len(), names.len(), "generated names must be unique");

        for name in &names {
            assert!(
                !model.symbols.iter().any(|symbol| symbol.name == *name),
                "generated `{name}` collides with a model symbol"
            );
            assert!(!DEFAULT_GLOBALS.contains(&name.as_str()));
            assert!(!ROBLOX_GLOBALS.contains(&name.as_str()));
        }

        let prefixed = generator
            .generate_with_prefix("__R4_SELECT")
            .expect("the reserved prefix is a valid identifier");
        assert_ne!(
            prefixed, "__R4_SELECT",
            "the prefix collides with a model symbol"
        );
    }

    #[test]
    fn an_invalid_prefix_is_a_typed_error() {
        let (_, model) = analyze_with_tokens("return 1");
        let mut generator = NameGenerator::new(&model);

        for prefix in ["1x", "", "not an identifier"] {
            let error = generator
                .generate_with_prefix(prefix)
                .expect_err("the prefix cannot begin an identifier");
            assert_eq!(error.code(), "name-generator/invalid-prefix");
            assert_eq!(error.prefix(), prefix);
        }
    }
}
