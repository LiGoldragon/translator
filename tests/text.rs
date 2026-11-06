#![cfg(feature = "text")]
use datom_codec::{Budget, Datomizable};
use translator::{decode, encode};

#[derive(Debug, PartialEq, Datomizable, datom_codec::Composing)]
struct Pair(String, i64);

#[test]
fn text_roundtrip() {
    let value = Pair("Ada".into(), 42);
    let text = encode(&value);
    let mut budget = Budget { remaining: 32, reader: protos::ReaderBudget::try_from(32usize).unwrap(), depth: 0, maximum_depth: 8 };
    assert_eq!(decode::<Pair>(&text, &mut budget).unwrap(), value);
}

#[test]
fn text_decode_error() {
    let mut budget = Budget { remaining: 32, reader: protos::ReaderBudget::try_from(32usize).unwrap(), depth: 0, maximum_depth: 8 };
    assert!(decode::<Pair>("[ 1 ]", &mut budget).is_err());
}
