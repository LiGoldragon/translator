#![cfg(feature = "text")]
use translator::placement::{Directory, MetaflowName, Place, Registry};
use translator::registry::{decode, select, validate, RegistryError};
use datom_codec::Budget;
fn budget() -> Budget { Budget { remaining: 64, reader: protos::ReaderBudget { remaining: 64 }, depth: 0, maximum_depth: 16 } }
#[test] fn registry_text_selects_unique_place() { let r=decode("[ { alpha /srv/a } { beta /srv/b } ]", &mut budget()).unwrap(); assert_eq!(select(&r,"beta").unwrap().directory.0,"/srv/b"); }
#[test] fn duplicate_name_or_directory_refused() { let r=Registry(vec![Place{metaflow_name:MetaflowName("a".into()),directory:Directory("/x".into())},Place{metaflow_name:MetaflowName("a".into()),directory:Directory("/y".into())}]); assert!(matches!(validate(&r),Err(RegistryError::Invalid(_)))); }
