//! Conversion of serialized data into Lua modules.
//!
//! [`convert_data`] turns any [`serde::Serialize`] value (JSON, YAML, TOML or
//! application data) into a Lua module that returns the data as a table. It is
//! the library counterpart of darklua's `convert` command.

use std::borrow::Cow;
use std::fmt;

use serde::ser::{self, Serialize};

use crate::generator::{DenseLuaGenerator, LuaGenerator};
use crate::nodes::{
    Block, DecimalNumber, Expression, FieldExpression, FunctionCall, HexNumber, Identifier,
    ReturnStatement, StringExpression, TableEntry, TableExpression, TableFieldEntry,
    TableIndexEntry, TupleArguments,
};
use crate::process::utils::is_valid_identifier;

/// An error produced while converting data into Lua.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConvertDataError {
    message: Cow<'static, str>,
}

impl ConvertDataError {
    fn new(message: impl Into<Cow<'static, str>>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// Returns the error message.
    #[inline]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ConvertDataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ConvertDataError {}

impl ser::Error for ConvertDataError {
    fn custom<T: fmt::Display>(message: T) -> Self {
        ConvertDataError::new(message.to_string())
    }
}

/// Converts a serializable value into the equivalent Lua [`Expression`].
pub fn to_expression<T: Serialize>(value: &T) -> Result<Expression, ConvertDataError> {
    let mut serializer = Serializer {
        output: Expression::nil(),
        operation: Vec::new(),
        expression_stack: Vec::new(),
    };
    value.serialize(&mut serializer)?;
    Ok(serializer.output)
}

/// Converts a serializable value into a Lua module returning the data.
///
/// ```
/// # use r4::convert::convert_data;
/// let code = convert_data(vec![1, 2, 3]).unwrap();
/// assert_eq!(code, "return{1,2,3}");
/// ```
pub fn convert_data<T: Serialize>(value: T) -> Result<String, ConvertDataError> {
    let expression = to_expression(&value)?;

    let mut block = Block::default();
    block.set_last_statement(ReturnStatement::one(expression));

    let mut generator = DenseLuaGenerator::default();
    generator.write_block(&block);
    Ok(generator.into_string())
}

enum SerializeOperation {
    Table(Vec<TableEntry>),
    TableEntryKey,
    TableEntryValue,
}

struct Serializer {
    output: Expression,
    operation: Vec<SerializeOperation>,
    expression_stack: Vec<Expression>,
}

impl Serializer {
    fn process(&mut self, expression: Expression) -> Result<(), ConvertDataError> {
        if let Some(mut operation) = self.operation.pop() {
            let keep = match &mut operation {
                SerializeOperation::Table(entries) => {
                    entries.push(TableEntry::from_value(expression));
                    true
                }
                SerializeOperation::TableEntryKey => {
                    self.expression_stack.push(expression);
                    false
                }
                SerializeOperation::TableEntryValue => {
                    self.complete_table_entry(expression)?;
                    false
                }
            };

            if keep {
                self.operation.push(operation);
            }
        } else {
            self.output = expression;
        }
        Ok(())
    }

    /// Completes a table entry once its value is known: the key was pushed on
    /// the expression stack by [`SerializeOperation::TableEntryKey`].
    fn complete_table_entry(&mut self, value: Expression) -> Result<(), ConvertDataError> {
        let key = self
            .expression_stack
            .pop()
            .ok_or_else(|| ConvertDataError::new("a table key was expected"))?;

        let Some(SerializeOperation::Table(entries)) = self.operation.last_mut() else {
            return Err(ConvertDataError::new(
                "a table operation was expected to insert an entry",
            ));
        };

        match key {
            Expression::String(string) => match string
                .get_string_value()
                .filter(|name| is_valid_identifier(name))
            {
                Some(name) => entries.push(TableFieldEntry::new(name.to_owned(), value).into()),
                None => entries.push(TableIndexEntry::new(string, value).into()),
            },
            key => entries.push(TableIndexEntry::new(key, value).into()),
        }

        Ok(())
    }

    fn begin_table(&mut self, len: Option<usize>) {
        let mut entries = Vec::new();
        if let Some(len) = len {
            entries.reserve_exact(len);
        }
        self.operation.push(SerializeOperation::Table(entries));
    }

    fn close_table(&mut self) -> Result<(), ConvertDataError> {
        let Some(operation) = self.operation.pop() else {
            return Err(ConvertDataError::new(
                "a table operation was expected to close a table",
            ));
        };

        match operation {
            SerializeOperation::Table(entries) => {
                self.process(TableExpression::new(entries).into())
            }
            _ => Err(ConvertDataError::new(
                "a table operation was expected to close a table",
            )),
        }
    }

    fn begin_table_entry_key(&mut self) {
        self.operation.push(SerializeOperation::TableEntryKey);
    }

    fn begin_table_entry_value(&mut self) {
        self.operation.push(SerializeOperation::TableEntryValue);
    }
}

impl ser::Serializer for &mut Serializer {
    type Ok = ();
    type Error = ConvertDataError;

    type SerializeSeq = Self;
    type SerializeTuple = Self;
    type SerializeTupleStruct = Self;
    type SerializeTupleVariant = Self;
    type SerializeMap = Self;
    type SerializeStruct = Self;
    type SerializeStructVariant = Self;

    fn serialize_bool(self, value: bool) -> Result<(), ConvertDataError> {
        self.process(value.into())
    }

    fn serialize_i8(self, value: i8) -> Result<(), ConvertDataError> {
        self.serialize_i64(i64::from(value))
    }

    fn serialize_i16(self, value: i16) -> Result<(), ConvertDataError> {
        self.serialize_i64(i64::from(value))
    }

    fn serialize_i32(self, value: i32) -> Result<(), ConvertDataError> {
        self.serialize_i64(i64::from(value))
    }

    fn serialize_i64(self, value: i64) -> Result<(), ConvertDataError> {
        self.process(DecimalNumber::new_integer(value).into())
    }

    fn serialize_u8(self, value: u8) -> Result<(), ConvertDataError> {
        self.serialize_u64(u64::from(value))
    }

    fn serialize_u16(self, value: u16) -> Result<(), ConvertDataError> {
        self.serialize_u64(u64::from(value))
    }

    fn serialize_u32(self, value: u32) -> Result<(), ConvertDataError> {
        self.serialize_u64(u64::from(value))
    }

    fn serialize_u64(self, value: u64) -> Result<(), ConvertDataError> {
        match i64::try_from(value) {
            // Values that fit a Lua 5.3/5.4 integer are written exactly.
            Ok(integer) => self.process(DecimalNumber::new_integer(integer).into()),
            // Lua integers are signed 64-bit. A larger `u64` has no exact
            // representation, so fail instead of silently rounding through a
            // float.
            Err(_) => Err(ConvertDataError::new(format!(
                "unsigned integer {value} does not fit in a Lua 64-bit signed integer"
            ))),
        }
    }

    fn serialize_f32(self, value: f32) -> Result<(), ConvertDataError> {
        self.serialize_f64(f64::from(value))
    }

    fn serialize_f64(self, value: f64) -> Result<(), ConvertDataError> {
        self.process(DecimalNumber::new(value).into())
    }

    fn serialize_char(self, value: char) -> Result<(), ConvertDataError> {
        self.serialize_str(&value.to_string())
    }

    fn serialize_str(self, value: &str) -> Result<(), ConvertDataError> {
        self.process(StringExpression::from_value(value).into())
    }

    fn serialize_bytes(self, value: &[u8]) -> Result<(), ConvertDataError> {
        let arguments = value
            .iter()
            .fold(TupleArguments::default(), |arguments, byte| {
                arguments.with_argument(HexNumber::new(u64::from(*byte), false))
            });
        let call = FunctionCall::new(
            FieldExpression::new(Identifier::new("string"), Identifier::new("char")).into(),
            arguments.into(),
            None,
        );
        self.process(call.into())
    }

    fn serialize_none(self) -> Result<(), ConvertDataError> {
        self.process(Expression::nil())
    }

    fn serialize_some<T>(self, value: &T) -> Result<(), ConvertDataError>
    where
        T: ?Sized + Serialize,
    {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<(), ConvertDataError> {
        self.process(Expression::nil())
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<(), ConvertDataError> {
        self.serialize_unit()
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<(), ConvertDataError> {
        self.serialize_str(variant)
    }

    fn serialize_newtype_struct<T>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<(), ConvertDataError>
    where
        T: ?Sized + Serialize,
    {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T>(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<(), ConvertDataError>
    where
        T: ?Sized + Serialize,
    {
        self.begin_table(Some(1));
        self.begin_table_entry_key();
        variant.serialize(&mut *self)?;
        self.begin_table_entry_value();
        value.serialize(&mut *self)?;
        self.close_table()
    }

    fn serialize_seq(self, len: Option<usize>) -> Result<Self::SerializeSeq, ConvertDataError> {
        self.begin_table(len);
        Ok(self)
    }

    fn serialize_tuple(self, len: usize) -> Result<Self::SerializeTuple, ConvertDataError> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleStruct, ConvertDataError> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleVariant, ConvertDataError> {
        self.begin_table(Some(1));
        self.begin_table_entry_key();
        variant.serialize(&mut *self)?;
        self.begin_table_entry_value();
        self.begin_table(Some(len));
        Ok(self)
    }

    fn serialize_map(self, len: Option<usize>) -> Result<Self::SerializeMap, ConvertDataError> {
        self.begin_table(len);
        Ok(self)
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStruct, ConvertDataError> {
        self.serialize_map(Some(len))
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStructVariant, ConvertDataError> {
        self.begin_table(Some(1));
        self.begin_table_entry_key();
        variant.serialize(&mut *self)?;
        self.begin_table_entry_value();
        self.begin_table(Some(len));
        Ok(self)
    }
}

impl ser::SerializeSeq for &mut Serializer {
    type Ok = ();
    type Error = ConvertDataError;

    fn serialize_element<T>(&mut self, value: &T) -> Result<(), ConvertDataError>
    where
        T: ?Sized + Serialize,
    {
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<(), ConvertDataError> {
        self.close_table()
    }
}

impl ser::SerializeTuple for &mut Serializer {
    type Ok = ();
    type Error = ConvertDataError;

    fn serialize_element<T>(&mut self, value: &T) -> Result<(), ConvertDataError>
    where
        T: ?Sized + Serialize,
    {
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<(), ConvertDataError> {
        self.close_table()
    }
}

impl ser::SerializeTupleStruct for &mut Serializer {
    type Ok = ();
    type Error = ConvertDataError;

    fn serialize_field<T>(&mut self, value: &T) -> Result<(), ConvertDataError>
    where
        T: ?Sized + Serialize,
    {
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<(), ConvertDataError> {
        self.close_table()
    }
}

impl ser::SerializeTupleVariant for &mut Serializer {
    type Ok = ();
    type Error = ConvertDataError;

    fn serialize_field<T>(&mut self, value: &T) -> Result<(), ConvertDataError>
    where
        T: ?Sized + Serialize,
    {
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<(), ConvertDataError> {
        self.close_table()?;
        self.close_table()
    }
}

impl ser::SerializeMap for &mut Serializer {
    type Ok = ();
    type Error = ConvertDataError;

    fn serialize_key<T>(&mut self, key: &T) -> Result<(), ConvertDataError>
    where
        T: ?Sized + Serialize,
    {
        self.begin_table_entry_key();
        key.serialize(&mut **self)
    }

    fn serialize_value<T>(&mut self, value: &T) -> Result<(), ConvertDataError>
    where
        T: ?Sized + Serialize,
    {
        self.begin_table_entry_value();
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<(), ConvertDataError> {
        self.close_table()
    }
}

impl ser::SerializeStruct for &mut Serializer {
    type Ok = ();
    type Error = ConvertDataError;

    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<(), ConvertDataError>
    where
        T: ?Sized + Serialize,
    {
        self.begin_table_entry_key();
        key.serialize(&mut **self)?;
        self.begin_table_entry_value();
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<(), ConvertDataError> {
        self.close_table()
    }
}

impl ser::SerializeStructVariant for &mut Serializer {
    type Ok = ();
    type Error = ConvertDataError;

    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<(), ConvertDataError>
    where
        T: ?Sized + Serialize,
    {
        self.begin_table_entry_key();
        key.serialize(&mut **self)?;
        self.begin_table_entry_value();
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<(), ConvertDataError> {
        self.close_table()?;
        self.close_table()
    }
}
