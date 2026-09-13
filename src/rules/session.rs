//! A transformation session owning a block and its semantic model.
//!
//! Semantic passes require a [`SemanticModel`] that matches the current AST.
//! Mutating the block with a model-less pass invalidates the model; using a
//! stale model silently produces wrong queries. [`Session`] removes that
//! footgun: it caches the model and re-analyzes lazily whenever the block was
//! changed, and [`Session::apply_semantic`] always hands passes a fresh model.

use std::convert::Infallible;
use std::fmt;

use crate::nodes::Block;
use crate::parser::{Parser, ParserError};
use crate::semantic::{SemanticAnalyzer, SemanticError, SemanticModel};

use crate::generator::{DenseLuaGenerator, LuaGenerator, ReadableLuaGenerator};

/// An error produced by a [`Session`].
#[derive(Debug)]
pub enum SessionError<E = Infallible> {
    /// The source code could not be parsed.
    Parse(ParserError),
    /// The block could not be analyzed.
    Semantic(SemanticError),
    /// A transformation pass failed.
    Pass(E),
}

impl<E: fmt::Display> fmt::Display for SessionError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionError::Parse(error) => write!(f, "parse error: {}", error),
            SessionError::Semantic(error) => write!(f, "semantic error: {}", error),
            SessionError::Pass(error) => write!(f, "pass error: {}", error),
        }
    }
}

impl<E: fmt::Debug + fmt::Display> std::error::Error for SessionError<E> {}

impl<E> From<ParserError> for SessionError<E> {
    fn from(error: ParserError) -> Self {
        SessionError::Parse(error)
    }
}

impl<E> From<SemanticError> for SessionError<E> {
    fn from(error: SemanticError) -> Self {
        SessionError::Semantic(error)
    }
}

/// Owns a block together with a lazily refreshed semantic model.
#[derive(Debug)]
pub struct Session {
    block: Block,
    model: Option<SemanticModel>,
}

impl Session {
    /// Parses the source and analyzes it.
    ///
    /// The AST is parsed without token data: semantic passes resolve
    /// identifiers and statements through the structural bindings recorded by
    /// the analyzer, not through source positions.
    pub fn parse(code: &str) -> Result<Self, SessionError> {
        let block = Parser::default().parse(code)?;
        Self::from_block(block)
    }

    /// Analyzes an existing block.
    pub fn from_block(block: Block) -> Result<Self, SessionError> {
        let model = SemanticAnalyzer::new().analyze(&block)?;
        Ok(Self {
            block,
            model: Some(model),
        })
    }

    /// Returns the current block.
    #[inline]
    pub fn block(&self) -> &Block {
        &self.block
    }

    /// Consumes the session and returns the block.
    #[inline]
    pub fn into_block(self) -> Block {
        self.block
    }

    /// Renders the current block with the readable generator.
    pub fn generate(&self) -> String {
        let mut generator = ReadableLuaGenerator::default();
        generator.write_block(&self.block);
        generator.into_string()
    }

    /// Renders the current block with the dense (minified) generator.
    ///
    /// Minification is a property of code generation, not of the AST: the
    /// dense generator emits only the semantic tree, so comments, whitespace
    /// and formatting trivia are dropped without any token-mutating rule.
    pub fn generate_minified(&self) -> String {
        let mut generator = DenseLuaGenerator::default();
        generator.write_block(&self.block);
        generator.into_string()
    }

    /// Renders the current block with a caller-provided generator.
    pub fn generate_with<G: LuaGenerator>(&self, mut generator: G) -> String {
        generator.write_block(&self.block);
        generator.into_string()
    }

    /// Applies a pass that does not need semantic information and invalidates
    /// the cached model.
    pub fn apply(&mut self, pass: impl FnOnce(&mut Block)) {
        pass(&mut self.block);
        self.model = None;
    }

    /// Returns a model for the current block, re-analyzing it first when it
    /// changed since the last analysis.
    pub fn model(&mut self) -> Result<&SemanticModel, SessionError> {
        self.refresh()?;
        Ok(self
            .model
            .as_ref()
            .expect("the model was built by the refresh"))
    }

    /// Applies a pass that needs semantic information, refreshing the model
    /// first. The cached model is invalidated afterwards: the pass may have
    /// changed the block, so the next query re-analyzes.
    pub fn apply_semantic<R>(
        &mut self,
        pass: impl FnOnce(&mut Block, &SemanticModel) -> R,
    ) -> Result<R, SessionError> {
        self.refresh()?;
        let result = {
            let model = self
                .model
                .as_ref()
                .expect("the model was built by the refresh");
            pass(&mut self.block, model)
        };
        self.model = None;
        Ok(result)
    }

    /// Applies a fallible pass that needs semantic information, refreshing the
    /// model first. The pass error is preserved in [`SessionError::Pass`]. The
    /// cached model is invalidated afterwards, even when the pass fails,
    /// because a failed pass may still have changed the block.
    pub fn apply_semantic_result<R, E>(
        &mut self,
        pass: impl FnOnce(&mut Block, &SemanticModel) -> Result<R, E>,
    ) -> Result<R, SessionError<E>> {
        self.refresh().map_err(SessionError::Semantic)?;
        let result = {
            let model = self
                .model
                .as_ref()
                .expect("the model was built by the refresh");
            pass(&mut self.block, model).map_err(SessionError::Pass)
        };
        self.model = None;
        result
    }

    fn refresh(&mut self) -> Result<(), SemanticError> {
        if self.model.is_none() {
            self.model = Some(SemanticAnalyzer::new().analyze(&self.block)?);
        }
        Ok(())
    }
}
