//! Structural equivalence of syntax trees.
//!
//! [`Equivalent`] compares two trees for *structural* equality: the same
//! statements, expressions, operators, identifiers, literals and inferred
//! structure, ignoring everything that is source presentation — trivia
//! (comments and whitespace), token source positions, and whether token data
//! was preserved at all.
//!
//! The comparison is exact, not a canonical rendering: it clones both trees,
//! erases every token with [`detokenize`](crate::process::detokenize) and
//! compares the resulting trees with the derived structural `PartialEq`. A
//! field that is not erased by detokenization takes part in the comparison, so
//! the relation can never report two structurally different trees as
//! equivalent. Semantic identities (bindings) are metadata and are already
//! excluded from node equality.
//!
//! ```
//! use r4::{Equivalent, Parser};
//!
//! let with_trivia = Parser::default()
//!     .preserve_tokens()
//!     .parse("local x = 1 -- keep\nreturn x")
//!     .unwrap();
//! let plain = Parser::default().parse("local x=1\nreturn x").unwrap();
//!
//! assert!(with_trivia.is_equivalent_to(&plain));
//!
//! let different = Parser::default().parse("local x = 2\nreturn x").unwrap();
//! assert!(!with_trivia.is_equivalent_to(&different));
//! ```

use crate::nodes::{Block, Expression, Statement};
use crate::process::{detokenize, detokenize_expression, detokenize_statement};

/// Structural equivalence between two syntax trees.
pub trait Equivalent {
    /// Returns whether `self` and `other` have the same structure and semantic
    /// content, ignoring trivia and all token data.
    fn is_equivalent_to(&self, other: &Self) -> bool;
}

impl Equivalent for Block {
    fn is_equivalent_to(&self, other: &Self) -> bool {
        if self == other {
            return true;
        }
        let mut left = self.clone();
        let mut right = other.clone();
        detokenize(&mut left);
        detokenize(&mut right);
        left == right
    }
}

impl Equivalent for Statement {
    fn is_equivalent_to(&self, other: &Self) -> bool {
        if self == other {
            return true;
        }
        let mut left = self.clone();
        let mut right = other.clone();
        detokenize_statement(&mut left);
        detokenize_statement(&mut right);
        left == right
    }
}

impl Equivalent for Expression {
    fn is_equivalent_to(&self, other: &Self) -> bool {
        if self == other {
            return true;
        }
        let mut left = self.clone();
        let mut right = other.clone();
        detokenize_expression(&mut left);
        detokenize_expression(&mut right);
        left == right
    }
}
