use std::fmt;

use crate::nodes::Token;

use super::binding::IdentityCell;
use super::{BindingId, Type, TypedIdentifier};

/// Represents an identifier (variable name).
pub struct Identifier {
    name: String,
    token: Option<Token>,
    binding: IdentityCell,
}

impl Clone for Identifier {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            token: self.token.clone(),
            binding: self.binding.clone(),
        }
    }
}

impl PartialEq for Identifier {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.token == other.token
    }
}

impl Eq for Identifier {}

impl fmt::Debug for Identifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("Identifier");
        debug.field("name", &self.name);
        debug.field("token", &self.token);
        if let Some(binding) = self.get_binding() {
            debug.field("binding", &binding);
        }
        debug.finish()
    }
}

impl Identifier {
    /// Creates a new identifier with the given name.
    pub fn new<S: Into<String>>(name: S) -> Self {
        Self {
            name: name.into(),
            token: None,
            binding: IdentityCell::empty(),
        }
    }

    /// Returns the semantic binding of this identifier, if it was analyzed.
    ///
    /// The binding is written by [`SemanticAnalyzer`](crate::semantic::SemanticAnalyzer)
    /// and survives cloning: a copied identifier still refers to the same
    /// symbol or label.
    #[inline]
    pub fn get_binding(&self) -> Option<BindingId> {
        self.binding.get().map(BindingId::from_raw)
    }

    /// Records the semantic binding of this identifier.
    #[inline]
    pub fn set_binding(&self, binding: BindingId) {
        self.binding.set(binding.raw());
    }

    /// Removes the semantic binding of this identifier.
    #[inline]
    pub fn clear_binding(&self) {
        self.binding.clear();
    }

    /// Converts this identifier into a typed identifier.
    pub fn with_type(self, r#type: impl Into<Type>) -> TypedIdentifier {
        TypedIdentifier::from(self).with_type(r#type.into())
    }

    /// Attaches token information to this identifier.
    pub fn with_token(mut self, token: Token) -> Self {
        self.token = Some(token);
        self
    }

    /// Sets the token information for this identifier.
    #[inline]
    pub fn set_token(&mut self, token: Token) {
        self.token = Some(token);
    }

    /// Returns a reference to the token information attached to this identifier, if any.
    #[inline]
    pub fn get_token(&self) -> Option<&Token> {
        self.token.as_ref()
    }

    /// Returns a mutable reference to the token information attached to this identifier, if any.
    #[inline]
    pub fn mutate_token(&mut self) -> Option<&mut Token> {
        self.token.as_mut()
    }

    /// Returns a mutable reference to the token information attached to this identifier.
    /// If no token is attached, it creates one from the name.
    pub(crate) fn mutate_or_insert_token(&mut self) -> &mut Token {
        if self.token.is_none() {
            let name = self.get_name().to_owned();
            self.token = Some(Token::from_content(name));
        }
        self.token.as_mut().unwrap()
    }

    /// Returns a reference to the name of this identifier.
    #[inline]
    pub fn get_name(&self) -> &String {
        &self.name
    }

    /// Returns a mutable reference to the name of this identifier.
    #[inline]
    pub fn mutate_name(&mut self) -> &mut String {
        &mut self.name
    }

    /// Changes the name of this identifier.
    ///
    /// If token information is attached, it's updated to reflect the new name.
    #[inline]
    pub fn set_name<IntoString: Into<String>>(&mut self, name: IntoString) {
        let name = name.into();
        if let Some(token) = &mut self.token {
            token.replace_with_content(name.clone());
        }
        self.name = name;
    }

    /// Consumes the identifier and returns just the name as a String.
    #[inline]
    pub fn into_name(self) -> String {
        self.name
    }

    super::impl_token_fns!(iter = [token]);
}

impl<IntoString: Into<String>> From<IntoString> for Identifier {
    fn from(identifier: IntoString) -> Self {
        Self {
            name: identifier.into(),
            token: None,
            binding: IdentityCell::empty(),
        }
    }
}

#[cfg(test)]
mod test {
    use crate::nodes::Position;

    use super::*;

    #[test]
    fn set_name_replaces_the_token_content() {
        let token = Token::new_with_line(7, 10, 1);
        let mut identifier = Identifier::new("var").with_token(token);

        identifier.set_name("newVar");

        assert_eq!(
            identifier.get_token().unwrap(),
            &Token::from_position(Position::LineNumber {
                line_number: 1,
                content: "newVar".into(),
            })
        );
    }
}
