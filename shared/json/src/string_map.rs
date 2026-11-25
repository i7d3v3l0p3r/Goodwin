//! Serde adapter for typed JSON objects with owned string keys.

use alloc::{collections::BTreeMap, string::String};
use core::{fmt, marker::PhantomData};

use serde::{
    Deserialize, Deserializer,
    de::{MapAccess, Visitor},
};

pub fn deserialize<'de, D, T>(deserializer: D) -> Result<BTreeMap<String, T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    // serde-json-core map keys only support deserialize_str. Decode through a
    // string visitor, then own the key before moving on to the next entry.
    struct Entries<T>(PhantomData<T>);

    impl<'de, T: Deserialize<'de>> Visitor<'de> for Entries<T> {
        type Value = BTreeMap<String, T>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a JSON object")
        }

        fn visit_map<A: MapAccess<'de>>(self, mut source: A) -> Result<Self::Value, A::Error> {
            let mut entries = BTreeMap::new();
            while let Some((Key(key), entry)) = source.next_entry()? {
                entries.insert(key, entry);
            }
            Ok(entries)
        }
    }

    deserializer.deserialize_map(Entries(PhantomData))
}

struct Key(String);

impl<'de> Deserialize<'de> for Key {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct KeyVisitor;

        impl Visitor<'_> for KeyVisitor {
            type Value = Key;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an object key")
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Key, E> {
                Ok(Key(value.into()))
            }
        }

        deserializer.deserialize_str(KeyVisitor)
    }
}
