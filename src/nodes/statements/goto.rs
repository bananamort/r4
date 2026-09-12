use crate::nodes::{Identifier, Token};

/// Tokens associated with a goto statement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GotoTokens {
    pub r#goto: Token,
}

impl GotoTokens {
    super::impl_token_fns!(target = [r#goto]);
}

/// Represents a goto statement, such as `goto label`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GotoStatement {
    label: Identifier,
    tokens: Option<GotoTokens>,
}

impl GotoStatement {
    /// Creates a new goto statement targeting the given label.
    pub fn new(label: impl Into<Identifier>) -> Self {
        Self {
            label: label.into(),
            tokens: None,
        }
    }

    /// Creates a new goto statement from a label name.
    pub fn from_name<S: Into<String>>(name: S) -> Self {
        Self::new(Identifier::new(name))
    }

    /// Sets the tokens for this goto statement.
    pub fn with_tokens(mut self, tokens: GotoTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }

    /// Sets the tokens for this goto statement.
    #[inline]
    pub fn set_tokens(&mut self, tokens: GotoTokens) {
        self.tokens = Some(tokens);
    }

    /// Returns the tokens for this goto statement, if any.
    #[inline]
    pub fn get_tokens(&self) -> Option<&GotoTokens> {
        self.tokens.as_ref()
    }

    /// Returns a mutable reference to the tokens, if any.
    #[inline]
    pub fn mutate_tokens(&mut self) -> Option<&mut GotoTokens> {
        self.tokens.as_mut()
    }

    /// Returns the label targeted by this goto statement.
    #[inline]
    pub fn get_label(&self) -> &Identifier {
        &self.label
    }

    /// Returns a mutable reference to the targeted label.
    #[inline]
    pub fn mutate_label(&mut self) -> &mut Identifier {
        &mut self.label
    }

    /// Returns the name of the targeted label.
    #[inline]
    pub fn get_label_name(&self) -> &str {
        self.label.get_name()
    }

    /// Returns a mutable reference to the first token for this statement, creating it if missing.
    pub fn mutate_first_token(&mut self) -> &mut Token {
        if self.tokens.is_none() {
            self.tokens = Some(GotoTokens {
                r#goto: Token::from_content("goto"),
            });
        }
        &mut self.tokens.as_mut().unwrap().r#goto
    }

    /// Returns a mutable reference to the last token for this statement,
    /// creating it if missing.
    pub fn mutate_last_token(&mut self) -> &mut Token {
        self.label.mutate_or_insert_token()
    }

    super::impl_token_fns!(target = [label] iter = [tokens]);
}
