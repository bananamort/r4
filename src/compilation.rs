//! A unified compilation container.
//!
//! [`Compilation`] is the front door of the library. It owns parsed and
//! analyzed syntax trees, collects structured [`Diagnostic`]s and exposes
//! semantic queries. It is deliberately a thin orchestrator: parsing is
//! delegated to [`Parser`], analysis to [`SemanticAnalyzer`], and module
//! resolution to [`ModuleResolver`]. It never implements scoping, symbol
//! resolution or control flow itself, so the individual layers stay
//! independently usable.

use crate::diagnostic::{line_position_span, Diagnostic, LinePositionSpan, TextSpan};
use crate::modules::{ModuleProvider, ModuleResolver, ResolvedModules};
use crate::nodes::{Block, Identifier};
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

/// A parsed and analyzed syntax tree together with its source.
#[derive(Clone, Debug)]
pub struct SyntaxTree {
    source: String,
    block: Block,
    model: SemanticModel,
}

impl SyntaxTree {
    /// Returns the source text this tree was parsed from.
    #[inline]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Returns the syntax tree.
    #[inline]
    pub fn block(&self) -> &Block {
        &self.block
    }

    /// Returns the semantic model of [`Self::block`].
    #[inline]
    pub fn model(&self) -> &SemanticModel {
        &self.model
    }

    /// Converts a byte span into a one-based line/column span using this tree's
    /// source.
    pub fn location(&self, span: TextSpan) -> LinePositionSpan {
        line_position_span(&self.source, span)
    }
}

/// A container of parsed and analyzed trees.
///
/// ```
/// use r4::Compilation;
///
/// let mut compilation = Compilation::new();
/// let tree = compilation.add_tree("local x = 1\nreturn x").expect("valid code");
/// assert_eq!(compilation.tree(tree).unwrap().block().get_last_statement().is_some(), true);
/// assert!(compilation.diagnostics().is_empty());
/// ```
#[derive(Default)]
pub struct Compilation {
    parser: Parser,
    options: SemanticOptions,
    trees: Vec<SyntaxTree>,
    diagnostics: Vec<Diagnostic>,
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
    /// On failure the corresponding [`Diagnostic`] is recorded and returned.
    pub fn add_tree(&mut self, source: impl Into<String>) -> Result<TreeId, Diagnostic> {
        let source = source.into();

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

        let id = TreeId(self.trees.len() as u32);
        self.trees.push(SyntaxTree {
            source,
            block,
            model,
        });
        Ok(id)
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

    /// Resolves an identifier occurrence in the given tree to the entity it
    /// binds to.
    pub fn binding_of(&self, tree: TreeId, identifier: &Identifier) -> Option<Binding> {
        self.model(tree)?.binding_of(identifier)
    }

    /// Resolves the module graph starting from `root` using `provider`.
    ///
    /// The root tree is cloned into the returned graph; the compilation keeps
    /// its own copy. Any error is recorded as a [`Diagnostic`] before it is
    /// returned.
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

        match ModuleResolver::new(provider).resolve(block, model, ROOT_KEY) {
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
            .finish()
    }
}
