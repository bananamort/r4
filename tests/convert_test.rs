//! Tests for serialized-data conversion.

use std::collections::BTreeMap;

use r4::convert::{convert_data, to_expression};
use r4::nodes::Expression;

#[test]
fn converts_scalars() {
    assert_eq!(convert_data(42i32).unwrap(), "return 42");
    assert_eq!(convert_data(true).unwrap(), "return true");
    assert_eq!(convert_data("hello").unwrap(), "return'hello'");
    assert_eq!(convert_data(Option::<i32>::None).unwrap(), "return nil");
}

#[test]
fn converts_sequences_and_tuples() {
    assert_eq!(convert_data(vec![1, 2, 3]).unwrap(), "return{1,2,3}");
    assert_eq!(convert_data((1, "two")).unwrap(), "return{1,'two'}");
}

#[test]
fn converts_maps_with_identifier_and_index_keys() {
    let mut map = BTreeMap::new();
    map.insert("valid_name", 1);
    map.insert("not valid", 2);
    let code = convert_data(map).unwrap();
    assert!(code.contains("valid_name=1"), "{code}");
    assert!(code.contains("['not valid']=2"), "{code}");
}

#[test]
fn converts_nested_structures() {
    #[derive(serde::Serialize)]
    struct Data {
        name: &'static str,
        values: Vec<i32>,
    }

    let code = convert_data(Data {
        name: "test",
        values: vec![1, 2],
    })
    .unwrap();
    assert_eq!(code, "return{name='test',values={1,2}}");
}

#[test]
fn to_expression_produces_a_return_value() {
    let expression = to_expression(&vec![1, 2]).unwrap();
    assert!(matches!(expression, Expression::Table(_)));
}

#[test]
fn converts_externally_tagged_enums() {
    #[derive(serde::Serialize)]
    enum Shape {
        Empty,
        Circle(f64),
        Point { x: i32, y: i32 },
    }

    assert_eq!(convert_data(Shape::Empty).unwrap(), "return'Empty'");
    assert_eq!(
        convert_data(Shape::Circle(1.5)).unwrap(),
        "return{Circle=1.5}"
    );
    assert_eq!(
        convert_data(Shape::Point { x: 1, y: 2 }).unwrap(),
        "return{Point={x=1,y=2}}"
    );
}

#[test]
fn malformed_serialize_sequences_error_with_a_typed_error() {
    use serde::ser::SerializeMap;

    // A serializer that emits a map value without a preceding key breaks the
    // serializer's invariants and must be reported, not silently accepted.
    struct ValueWithoutKey;

    impl serde::Serialize for ValueWithoutKey {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            let mut map = serializer.serialize_map(Some(1))?;
            map.serialize_value(&1)?;
            map.end()
        }
    }

    let error = convert_data(ValueWithoutKey).expect_err("a value without a key is invalid");
    assert_eq!(error.message(), "a table key was expected");
    assert!(!error.to_string().is_empty());
}

#[test]
fn custom_serialize_errors_are_reported() {
    struct Failing;

    impl serde::Serialize for Failing {
        fn serialize<S: serde::Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("custom failure"))
        }
    }

    let error = convert_data(Failing).expect_err("a custom error is reported");
    assert_eq!(error.message(), "custom failure");
}

#[test]
fn converts_every_scalar_serialize_method() {
    assert_eq!(convert_data(1i8).expect("i8"), "return 1");
    assert_eq!(convert_data(1i16).expect("i16"), "return 1");
    assert_eq!(convert_data(1i32).expect("i32"), "return 1");
    assert_eq!(convert_data(1i64).expect("i64"), "return 1");
    assert_eq!(convert_data(1u8).expect("u8"), "return 1");
    assert_eq!(convert_data(1u16).expect("u16"), "return 1");
    assert_eq!(convert_data(1u32).expect("u32"), "return 1");
    assert_eq!(convert_data(1u64).expect("u64"), "return 1");
    // Values beyond f64's exact range must keep every digit.
    assert_eq!(
        convert_data(9_007_199_254_740_993i64).expect("large i64"),
        "return 9007199254740993"
    );
    assert_eq!(
        convert_data(i64::MAX).expect("i64::MAX"),
        format!("return {}", i64::MAX)
    );
    assert!(!convert_data(1.5f32).expect("f32").is_empty());
    assert!(!convert_data(1.5f64).expect("f64").is_empty());
    assert!(!convert_data('c').expect("char").is_empty());
    assert!(!convert_data("text").expect("str").is_empty());
}

#[test]
fn converts_map_keys_and_renders_errors() {
    use r4::convert::ConvertDataError;
    use serde::ser::Error as _;

    // A valid identifier key becomes a field; anything else an index entry.
    let map = BTreeMap::from([("valid".to_owned(), 1i64), ("not a name".to_owned(), 2i64)]);
    let output = convert_data(map).expect("map converts");
    assert!(output.contains("valid"));
    assert!(output.contains("not a name"));

    // Non-string keys become index entries.
    let numeric = BTreeMap::from([(1i64, "a"), (2i64, "b")]);
    let output = convert_data(numeric).expect("map converts");
    assert!(output.contains("[1]"));

    let expression =
        to_expression(&BTreeMap::from([("k".to_owned(), 1i64)])).expect("expression converts");
    assert!(!format!("{expression:?}").is_empty());

    let error = ConvertDataError::custom("boom");
    assert_eq!(error.message(), "boom");
    assert_eq!(error.to_string(), "boom");
}

#[test]
fn serializes_bytes_through_the_serializer() {
    struct Bytes(&'static [u8]);

    impl serde::Serialize for Bytes {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.serialize_bytes(self.0)
        }
    }

    let output = convert_data(Bytes(b"ABC")).expect("bytes convert");
    assert!(output.contains("string.char"), "{output}");
}

#[test]
fn serializes_every_fallback_serialize_method() {
    struct Unit;
    impl serde::Serialize for Unit {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.serialize_unit()
        }
    }

    struct UnitStruct;
    impl serde::Serialize for UnitStruct {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.serialize_unit_struct("UnitStruct")
        }
    }

    struct Newtype(i32);
    impl serde::Serialize for Newtype {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.serialize_newtype_struct("Newtype", &self.0)
        }
    }

    struct UnitVariant;
    impl serde::Serialize for UnitVariant {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.serialize_unit_variant("Enum", 0, "Unit")
        }
    }

    struct NewtypeVariant(i32);
    impl serde::Serialize for NewtypeVariant {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.serialize_newtype_variant("Enum", 0, "Newtype", &self.0)
        }
    }

    struct TupleVariant(i32, i32);
    impl serde::Serialize for TupleVariant {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            use serde::ser::SerializeTupleVariant;
            let mut variant = serializer.serialize_tuple_variant("Enum", 0, "Tuple", 2)?;
            variant.serialize_field(&self.0)?;
            variant.serialize_field(&self.1)?;
            variant.end()
        }
    }

    struct TupleStruct(i32, i32);
    impl serde::Serialize for TupleStruct {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            use serde::ser::SerializeTupleStruct;
            let mut tuple = serializer.serialize_tuple_struct("TupleStruct", 2)?;
            tuple.serialize_field(&self.0)?;
            tuple.serialize_field(&self.1)?;
            tuple.end()
        }
    }

    struct Named;
    impl serde::Serialize for Named {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            use serde::ser::SerializeStruct;
            let mut structure = serializer.serialize_struct("Named", 1)?;
            structure.serialize_field("value", &1)?;
            structure.end()
        }
    }

    assert_eq!(convert_data(Unit).expect("unit"), "return nil");
    assert_eq!(convert_data(UnitStruct).expect("unit struct"), "return nil");
    assert_eq!(convert_data(Newtype(1)).expect("newtype"), "return 1");
    assert!(!convert_data(Some(1i32)).expect("some").is_empty());
    assert!(!convert_data(UnitVariant).expect("unit variant").is_empty());
    assert!(!convert_data(NewtypeVariant(1))
        .expect("newtype variant")
        .is_empty());
    assert!(!convert_data(TupleVariant(1, 2))
        .expect("tuple variant")
        .is_empty());
    assert!(!convert_data(TupleStruct(1, 2))
        .expect("tuple struct")
        .is_empty());
    assert!(!convert_data(Named).expect("named struct").is_empty());
}
