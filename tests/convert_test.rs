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
