//! The collection of nodes used for the Lua abstract syntax tree.

mod arguments;
mod attributes;
mod binding;
mod block;
mod expressions;
mod function_body;
mod function_call;
mod identifier;
mod literal_expression;
mod statements;
mod token;
mod type_id;
mod typed_identifier;
mod types;
mod variable;

pub use arguments::*;
pub use attributes::*;
pub use binding::*;
pub use block::*;
pub use expressions::*;
pub use function_body::*;
pub use function_call::*;
pub use identifier::*;
pub use literal_expression::*;
pub use statements::*;
pub use token::*;
pub use type_id::*;
pub use typed_identifier::*;
pub use types::*;
pub use variable::*;

macro_rules! impl_token_fns {
    (
        target = [ $( $field:ident ),* $(,)? ]
        $(iter = [ $( $iter_field:ident),* $(,)? ])?
        $(iter_flatten = [ $( $iter_flatten_field:ident),* $(,)? ])?
        $(children = [ $( $child_field:ident),* $(,)? ])?
    ) => {
        /// Clears all comments from the tokens in this node.
        pub fn clear_comments(&mut self) {
            $(
                self.$field.clear_comments();
            )*
            $($(
                for token in self.$iter_field.iter_mut() {
                    token.clear_comments();
                }
            )*)?
            $($(
                for token in self.$iter_flatten_field.iter_mut().flatten() {
                    token.clear_comments();
                }
            )*)?
            $($(
                for child in self.$child_field.iter_mut() {
                    child.clear_comments();
                }
            )*)?
        }

        /// Clears all whitespaces information from the tokens in this node.
        pub fn clear_whitespaces(&mut self) {
            $(
                self.$field.clear_whitespaces();
            )*
            $($(
                for token in self.$iter_field.iter_mut() {
                    token.clear_whitespaces();
                }
            )*)?
            $($(
                for token in self.$iter_flatten_field.iter_mut().flatten() {
                    token.clear_whitespaces();
                }
            )*)?
            $($(
                for child in self.$child_field.iter_mut() {
                    child.clear_whitespaces();
                }
            )*)?
        }

                pub(crate) fn replace_referenced_tokens(&mut self, code: &str) {
            $(
                self.$field.replace_referenced_tokens(code);
            )*
            $($(
                for token in self.$iter_field.iter_mut() {
                    token.replace_referenced_tokens(code);
                }
            )*)?
            $($(
                for token in self.$iter_flatten_field.iter_mut().flatten() {
                    token.replace_referenced_tokens(code);
                }
            )*)?
            $($(
                for child in self.$child_field.iter_mut() {
                    child.replace_referenced_tokens(code);
                }
            )*)?
        }

                pub(crate) fn shift_token_line(&mut self, amount: isize) {
            $(
                self.$field.shift_token_line(amount);
            )*
            $($(
                for token in self.$iter_field.iter_mut() {
                    token.shift_token_line(amount);
                }
            )*)?
            $($(
                for token in self.$iter_flatten_field.iter_mut().flatten() {
                    token.shift_token_line(amount);
                }
            )*)?
            $($(
                for child in self.$child_field.iter_mut() {
                    child.shift_token_line(amount);
                }
            )*)?
        }

                pub(crate) fn filter_comments(&mut self, filter: impl Fn(&crate::nodes::Trivia) -> bool) {
            $(
                self.$field.filter_comments(&filter);
            )*
            $($(
                for token in self.$iter_field.iter_mut() {
                    token.filter_comments(&filter);
                }
            )*)?
            $($(
                for token in self.$iter_flatten_field.iter_mut().flatten() {
                    token.filter_comments(&filter);
                }
            )*)?
            $($(
                for child in self.$child_field.iter_mut() {
                    child.filter_comments(&filter);
                }
            )*)?
        }

        /// Removes every token from this node and its contained token-bearing
        /// children.
        ///
        /// Direct token fields are erased, iterable token containers are
        /// emptied, and the listed `children` are detokenized recursively.
        /// The node keeps its structure, so two trees that differ only in
        /// trivia, source positions or token presence become equal. This backs
        /// [`Equivalent::is_equivalent_to`](crate::Equivalent::is_equivalent_to).
                pub fn remove_tokens(&mut self) {
            $(
                self.$field.remove_tokens();
            )*
            $($(
                $crate::nodes::RemoveTokenData::remove_token_data(&mut self.$iter_field);
            )*)?
            $($(
                $crate::nodes::RemoveTokenData::remove_token_data(&mut self.$iter_flatten_field);
            )*)?
            $($(
                for child in self.$child_field.iter_mut() {
                    child.remove_tokens();
                }
            )*)?
        }

        /// Passes every token attached to this node to `f`.
        ///
        /// Direct token fields and iterable token containers are visited, and
        /// the listed `children` are visited recursively. Use [`collect_trivia`](crate::process::collect_trivia)
        /// for a deduplicated, source-ordered view of a tree's trivia.
                pub fn for_each_token<F: FnMut(&crate::nodes::Token)>(&self, f: &mut F) {
            $(
                self.$field.for_each_token(f);
            )*
            $($(
                for token in self.$iter_field.iter() {
                    token.for_each_token(f);
                }
            )*)?
            $($(
                for token in self.$iter_flatten_field.iter().flatten() {
                    token.for_each_token(f);
                }
            )*)?
            $($(
                for child in self.$child_field.iter() {
                    child.for_each_token(f);
                }
            )*)?
        }
    };

    (
        iter = [$($field:ident),*]
        children = [$($child_field:ident),*]
    ) => {
        $crate::nodes::impl_token_fns!(
            target = []
            iter = [ $($field,)* ]
            children = [ $($child_field,)* ]
        );
    };

    (
        iter = [$($field:ident),*]
        iter_flatten = [$($flatten_field:ident),*]
        children = [$($child_field:ident),*]
    ) => {
        $crate::nodes::impl_token_fns!(
            target = []
            iter = [ $($field,)* ]
            iter_flatten = [ $($flatten_field,)* ]
            children = [ $($child_field,)* ]
        );
    };

    (
        children = [$($child_field:ident),*]
    ) => {
        $crate::nodes::impl_token_fns!(
            target = []
            iter = []
            children = [ $($child_field,)* ]
        );
    };

    (
        iter = [$($field:ident),*]
        iter_flatten = [$($flatten_field:ident),*]
    ) => {
        $crate::nodes::impl_token_fns!(
            target = []
            iter = [ $($field,)* ]
            iter_flatten = [ $($flatten_field,)* ]
        );
    };

    (
        iter = [$($field:ident),*]
    ) => {
        $crate::nodes::impl_token_fns!(
            target = []
            iter = [ $($field,)* ]
            iter_flatten = []
        );
    };

    (
        iter_flatten = [$($field:ident),*]
    ) => {
        $crate::nodes::impl_token_fns!(
            target = []
            iter = []
            iter_flatten = [ $($field,)* ]
        );
    };
}

pub(crate) use impl_token_fns;

/// Removes token data from a token container.
///
/// This is the field-level operation behind
/// [`remove_tokens`](crate::nodes::Block::remove_tokens): a token container
/// field is emptied so that a tree parsed with token data compares equal to the
/// same tree parsed without it. [`Option`] containers become `None` and [`Vec`]
/// containers are cleared.
pub trait RemoveTokenData {
    /// Removes the token data from `self`.
    fn remove_token_data(&mut self);
}

impl<T> RemoveTokenData for Option<T> {
    fn remove_token_data(&mut self) {
        *self = None;
    }
}

impl<T> RemoveTokenData for Vec<T> {
    fn remove_token_data(&mut self) {
        self.clear();
    }
}
