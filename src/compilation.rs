//! A unified compilation container.
//!
//! [`Compilation`] is the front door of the library. It owns parsed and
//! analyzed syntax trees, collects structured [`Diagnostic`]s and exposes
//! semantic queries. It is deliberately a thin orchestrator: parsing is
//! delegated to [`Parser`], analysis to [`SemanticAnalyzer`], and module
//! resolution to [`ModuleResolver`]. It never implements scoping, symbol
//! resolution or control flow itself, so the individual layers stay
//! independently usable.
//!
//! Analysis is incremental: parsed and analyzed sources are kept in a
//! content-addressed [`AnalysisCache`](crate::modules::AnalysisCache), so
//! adding or updating a tree whose source is unchanged reuses the previous
//! analysis, and resolving a module graph reuses the analyses of unchanged
//! dependencies.

use std::sync::Arc;

use crate::diagnostic::{
    line_position_span, Diagnostic, DiagnosticFormatError, DiagnosticFormatter,
    DiagnosticQueryError, LinePositionSpan, TextSpan,
};
use crate::modules::{
    AnalysisCache, AnalyzedModule, ModuleProvider, ModuleResolver, ResolvedModules,
};
use crate::nodes::{Block, Identifier, Token};
use crate::parser::Parser;
use crate::semantic::{Binding, SemanticAnalyzer, SemanticModel, SemanticOptions};

/// The key used for the root tree when resolving a module graph.
const ROOT_KEY: &str = "<root>";

/// Identifies a tree inside a [`Compilation`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TreeId(pub u32);

impl TreeId {
    /// Returns this identifier as a `usize` index.
    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// A parsed and analyzed syntax tree.
///
/// Trees with identical source share their analysis, so cloning a
/// [`SyntaxTree`] is cheap.
#[derive(Clone, Debug)]
pub struct SyntaxTree {
    inner: Arc<AnalyzedModule>,
}

impl SyntaxTree {
    /// Returns the source text this tree was parsed from.
    #[inline]
    pub fn source(&self) -> &str {
        &self.inner.source
    }

    /// Returns the syntax tree.
    #[inline]
    pub fn block(&self) -> &Block {
        &self.inner.block
    }

    /// Returns the semantic model of [`Self::block`].
    #[inline]
    pub fn model(&self) -> &SemanticModel {
        &self.inner.model
    }

    /// Converts a byte span into a one-based line/column span using this tree's
    /// source.
    pub fn location(&self, span: TextSpan) -> LinePositionSpan {
        line_position_span(&self.inner.source, span)
    }
}

/// A container of parsed and analyzed trees.
///
/// ```
/// use r4::Compilation;
///
/// let mut compilation = Compilation::new();
/// let tree = compilation.add_tree("local x = 1\nreturn x").expect("valid code");
/// assert!(compilation.tree(tree).unwrap().block().get_last_statement().is_some());
/// assert!(compilation.diagnostics().is_empty());
/// ```
#[derive(Default)]
pub struct Compilation {
    parser: Parser,
    options: SemanticOptions,
    trees: Vec<SyntaxTree>,
    diagnostics: Vec<Diagnostic>,
    cache: AnalysisCache,
}

impl Compilation {
    /// Creates an empty compilation with the default parser and options.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an empty compilation using `parser`.
    pub fn with_parser(parser: Parser) -> Self {
        Self {
            parser,
            ..Self::default()
        }
    }

    /// Creates an empty compilation using `options`.
    pub fn with_options(options: SemanticOptions) -> Self {
        Self {
            options,
            ..Self::default()
        }
    }

    /// Replaces the parser.
    pub fn set_parser(&mut self, parser: Parser) {
        self.parser = parser;
    }

    /// Returns the configured parser.
    #[inline]
    pub fn parser(&self) -> &Parser {
        &self.parser
    }

    /// Replaces the semantic options.
    pub fn set_options(&mut self, options: SemanticOptions) {
        self.options = options;
    }

    /// Returns the configured semantic options.
    #[inline]
    pub fn options(&self) -> &SemanticOptions {
        &self.options
    }

    /// Parses and analyzes `source`, storing the tree on success.
    ///
    /// Identical sources reuse a cached analysis. On failure the corresponding
    /// [`Diagnostic`] is recorded and returned.
    pub fn add_tree(&mut self, source: impl Into<String>) -> Result<TreeId, Diagnostic> {
        let source = source.into();
        let analyzed = match self.cache.get(&source) {
            Some(cached) => cached,
            None => self.analyze(source)?,
        };
        Ok(self.push(analyzed))
    }

    /// Replaces the source of an existing tree, analyzing only when it changed.
    ///
    /// The tree keeps its identifier. On failure the diagnostic is recorded and
    /// the previous tree is left untouched.
    pub fn update_tree(&mut self, id: TreeId, source: impl Into<String>) -> Result<(), Diagnostic> {
        let source = source.into();
        if self.tree(id).map(SyntaxTree::source) == Some(source.as_str()) {
            return Ok(());
        }
        let analyzed = match self.cache.get(&source) {
            Some(cached) => cached,
            None => self.analyze(source)?,
        };
        self.trees[id.index()] = SyntaxTree { inner: analyzed };
        Ok(())
    }

    /// Re-analyzes a tree whose block was mutated in place.
    ///
    /// The source cache is not repopulated, because the mutated block no longer
    /// corresponds to its original source. On failure the diagnostic is
    /// recorded and the previous tree is left untouched.
    pub fn refresh_tree(&mut self, id: TreeId) -> Result<(), Diagnostic> {
        let Some(tree) = self.tree(id) else {
            let diagnostic = Diagnostic::error(format!("unknown tree identifier {:?}", id))
                .with_code("compile/unknown-tree");
            self.diagnostics.push(diagnostic.clone());
            return Err(diagnostic);
        };

        let source = tree.source().to_owned();
        let block = tree.block().clone();
        let model = match SemanticAnalyzer::with_options(self.options.clone()).analyze(&block) {
            Ok(model) => model,
            Err(error) => {
                let diagnostic = error.to_diagnostic().resolve_location(&source);
                self.diagnostics.push(diagnostic.clone());
                return Err(diagnostic);
            }
        };

        let analyzed = Arc::new(AnalyzedModule {
            source,
            block,
            model,
        });
        self.trees[id.index()] = SyntaxTree { inner: analyzed };
        Ok(())
    }

    fn analyze(&mut self, source: String) -> Result<Arc<AnalyzedModule>, Diagnostic> {
        let block = match self.parser.parse(&source) {
            Ok(block) => block,
            Err(error) => {
                let diagnostic = error.to_diagnostic();
                self.diagnostics.push(diagnostic.clone());
                return Err(diagnostic);
            }
        };

        let model = match SemanticAnalyzer::with_options(self.options.clone()).analyze(&block) {
            Ok(model) => model,
            Err(error) => {
                let diagnostic = error.to_diagnostic().resolve_location(&source);
                self.diagnostics.push(diagnostic.clone());
                return Err(diagnostic);
            }
        };

        Ok(self.cache.insert(AnalyzedModule {
            source,
            block,
            model,
        }))
    }

    fn push(&mut self, analyzed: Arc<AnalyzedModule>) -> TreeId {
        let id = TreeId(self.trees.len() as u32);
        self.trees.push(SyntaxTree { inner: analyzed });
        id
    }

    /// Returns the tree with the given identifier, if any.
    #[inline]
    pub fn tree(&self, id: TreeId) -> Option<&SyntaxTree> {
        self.trees.get(id.index())
    }

    /// Returns the tree's syntax tree, if the identifier is valid.
    #[inline]
    pub fn block(&self, id: TreeId) -> Option<&Block> {
        self.tree(id).map(SyntaxTree::block)
    }

    /// Returns the tree's semantic model, if the identifier is valid.
    #[inline]
    pub fn model(&self, id: TreeId) -> Option<&SemanticModel> {
        self.tree(id).map(SyntaxTree::model)
    }

    /// Returns the tree's source, if the identifier is valid.
    #[inline]
    pub fn source(&self, id: TreeId) -> Option<&str> {
        self.tree(id).map(SyntaxTree::source)
    }

    /// Returns a mutable reference to the tree's block for in-place edits.
    ///
    /// The tree's analysis is detached copy-on-write, so editing one tree never
    /// affects another tree that shares the same source. Call
    /// [`Self::refresh_tree`] afterwards to re-analyze the mutated block.
    pub fn block_mut(&mut self, id: TreeId) -> Option<&mut Block> {
        self.trees
            .get_mut(id.index())
            .map(|tree| &mut Arc::make_mut(&mut tree.inner).block)
    }

    /// Returns the number of trees.
    #[inline]
    pub fn len(&self) -> usize {
        self.trees.len()
    }

    /// Returns whether there are no trees.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.trees.is_empty()
    }

    /// Iterates over the tree identifiers in insertion order.
    pub fn iter_ids(&self) -> impl Iterator<Item = TreeId> {
        (0..self.trees.len() as u32).map(TreeId)
    }

    /// Returns the number of cached analyses.
    #[inline]
    pub fn cache_len(&self) -> usize {
        self.cache.len()
    }

    /// Removes every cached analysis.
    ///
    /// Existing trees keep their own shared analysis.
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }

    /// Returns the diagnostics recorded so far.
    #[inline]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Removes and returns the recorded diagnostics.
    pub fn take_diagnostics(&mut self) -> Vec<Diagnostic> {
        std::mem::take(&mut self.diagnostics)
    }

    /// Removes the recorded diagnostics.
    pub fn clear_diagnostics(&mut self) {
        self.diagnostics.clear();
    }

    /// Returns the recorded diagnostics whose span lies within `span`.
    ///
    /// Diagnostics without a span are not returned: they cannot be attributed
    /// to a source range.
    pub fn diagnostics_in(&self, span: TextSpan) -> Vec<&Diagnostic> {
        self.diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic
                    .span
                    .map(|candidate| span.start <= candidate.start && candidate.end <= span.end)
                    .unwrap_or(false)
            })
            .collect()
    }

    /// Returns the recorded diagnostics attached to a token's source span.
    ///
    /// Fails with [`DiagnosticQueryError::MissingSpan`] when the token has no
    /// span (the tree was parsed without token data).
    pub fn diagnostics_for_token(
        &self,
        token: &Token,
    ) -> Result<Vec<&Diagnostic>, DiagnosticQueryError> {
        let span = token.get_span().ok_or(DiagnosticQueryError::MissingSpan)?;
        Ok(self.diagnostics_in(span))
    }

    /// Renders a diagnostic with its source line and a caret annotation.
    pub fn format_diagnostic(
        &self,
        diagnostic: &Diagnostic,
        source: &str,
    ) -> Result<String, DiagnosticFormatError> {
        DiagnosticFormatter::new(source).render(diagnostic)
    }

    /// Resolves an identifier occurrence in the given tree to the entity it
    /// binds to.
    pub fn binding_of(&self, tree: TreeId, identifier: &Identifier) -> Option<Binding> {
        self.model(tree)?.binding_of(identifier)
    }

    /// Resolves the module graph starting from `root` using `provider`.
    ///
    /// The root tree is cloned into the returned graph; the compilation keeps
    /// its own copy. Unchanged dependency sources reuse the analysis cache, so
    /// re-resolving after editing a single file only re-analyzes that file. Any
    /// error is recorded as a [`Diagnostic`] before it is returned.
    pub fn resolve_modules<P: ModuleProvider>(
        &mut self,
        root: TreeId,
        provider: &P,
    ) -> Result<ResolvedModules, Diagnostic> {
        let block = match self.tree(root) {
            Some(tree) => tree.block().clone(),
            None => {
                let diagnostic = Diagnostic::error(format!("unknown tree identifier {:?}", root))
                    .with_code("compile/unknown-tree");
                self.diagnostics.push(diagnostic.clone());
                return Err(diagnostic);
            }
        };
        let model = self
            .model(root)
            .expect("the tree exists, so its model exists")
            .clone();

        match ModuleResolver::new(provider).resolve_with_cache(
            block,
            model,
            ROOT_KEY,
            &mut self.cache,
        ) {
            Ok(resolved) => Ok(resolved),
            Err(error) => {
                let diagnostic = error.to_diagnostic();
                self.diagnostics.push(diagnostic.clone());
                Err(diagnostic)
            }
        }
    }
}

impl std::fmt::Debug for Compilation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Compilation")
            .field("trees", &self.trees.len())
            .field("diagnostics", &self.diagnostics.len())
            .field("cache", &self.cache.len())
            .finish()
    }
}
