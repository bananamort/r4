use crate::nodes::{Identifier, Token};

/// Tokens associated with a label statement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LabelTokens {
    pub left_colons: Token,
    pub right_colons: Token,
}

impl LabelTokens {
    super::impl_token_fns!(target = [left_colons, right_colons]);
}

/// Represents a label statement, such as `::label::`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LabelStatement {
    label: Identifier,
    tokens: Option<LabelTokens>,
}

impl LabelStatement {
    /// Creates a new label statement with the given name.
    pub fn new(label: impl Into<Identifier>) -> Self {
        Self {
            label: label.into(),
            tokens: None,
        }
    }

    /// Creates a new label statement from a label name.
    pub fn from_name<S: Into<String>>(name: S) -> Self {
        Self::new(Identifier::new(name))
    }

    /// Sets the tokens for this label statement.
    pub fn with_tokens(mut self, tokens: LabelTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }

    /// Sets the tokens for this label statement.
    #[inline]
    pub fn set_tokens(&mut self, tokens: LabelTokens) {
        self.tokens = Some(tokens);
    }

    /// Returns the tokens for this label statement, if any.
    #[inline]
    pub fn get_tokens(&self) -> Option<&LabelTokens> {
        self.tokens.as_ref()
    }

    /// Returns a mutable reference to the tokens, if any.
    #[inline]
    pub fn mutate_tokens(&mut self) -> Option<&mut LabelTokens> {
        self.tokens.as_mut()
    }

    /// Returns the identifier naming this label.
    #[inline]
    pub fn get_label(&self) -> &Identifier {
        &self.label
    }

    /// Returns a mutable reference to the identifier naming this label.
    #[inline]
    pub fn mutate_label(&mut self) -> &mut Identifier {
        &mut self.label
    }

    /// Returns the name of this label.
    #[inline]
    pub fn get_label_name(&self) -> &str {
        self.label.get_name()
    }

    /// Returns a mutable reference to the first token for this statement, creating it if missing.
    pub fn mutate_first_token(&mut self) -> &mut Token {
        self.set_default_tokens();
        &mut self.tokens.as_mut().unwrap().left_colons
    }

    /// Returns a mutable reference to the last token for this statement,
    /// creating it if missing.
    pub fn mutate_last_token(&mut self) -> &mut Token {
        self.set_default_tokens();
        &mut self.tokens.as_mut().unwrap().right_colons
    }

    fn set_default_tokens(&mut self) {
        if self.tokens.is_none() {
            self.tokens = Some(LabelTokens {
                left_colons: Token::from_content("::"),
                right_colons: Token::from_content("::"),
            });
        }
    }

    super::impl_token_fns!(target = [label] iter = [tokens]);
}
