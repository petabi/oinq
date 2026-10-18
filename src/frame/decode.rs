//! Serde allocation guards layered over bincode's standard wire decoder.
// Bincode's serde adapter does not claim container allocations. Hide its size
// hints, validate declared counts before visiting, and charge elements before
// deserializing them. This bounds ordinary serde collections, including ZSTs.
use std::{cell::Cell, fmt, mem};

use serde::de::{
    self, DeserializeSeed, Deserializer, EnumAccess, MapAccess, SeqAccess, VariantAccess, Visitor,
};

pub(super) fn decode<'de, T: serde::Deserialize<'de>>(
    bytes: &'de [u8],
    limit: usize,
) -> Result<(T, usize), bincode::error::DecodeError> {
    // Bincode accepts only compile-time limits. Keep its byte accounting bounded
    // by the wire maximum; Guard enforces the caller's runtime allocation budget,
    // including collection counts that bincode's serde adapter does not claim.
    let budget = Cell::new(limit);
    bincode::serde::seed_decode_from_slice(
        Seed {
            seed: std::marker::PhantomData::<T>,
            budget: &budget,
        },
        bytes,
        bincode::config::standard().with_limit::<{ u32::MAX as usize }>(),
    )
}

fn charge<E: de::Error>(budget: &Cell<usize>, count: usize) -> Result<(), E> {
    let remaining = budget
        .get()
        .checked_sub(count)
        .ok_or_else(|| E::custom("frame decoding allocation budget exceeded"))?;
    budget.set(remaining);
    Ok(())
}

struct Guard<'a, D> {
    inner: D,
    budget: &'a Cell<usize>,
    allocation: bool,
}

struct Seed<'a, S> {
    seed: S,
    budget: &'a Cell<usize>,
}

impl<'de, S: DeserializeSeed<'de>> DeserializeSeed<'de> for Seed<'_, S> {
    type Value = S::Value;
    fn deserialize<D: Deserializer<'de>>(self, inner: D) -> Result<Self::Value, D::Error> {
        self.seed.deserialize(Guard {
            inner,
            budget: self.budget,
            allocation: false,
        })
    }
}

// All nested deserializers and visitors must retain the shared budget.
macro_rules! forward_deserialize {
    ($method:ident $(, $arg:ident: $ty:ty)*; $allocation:expr) => {
        fn $method<V: Visitor<'de>>(self, $($arg: $ty,)* visitor: V) -> Result<V::Value, Self::Error> {
            self.inner.$method($($arg,)* Guard {
                inner: visitor, budget: self.budget, allocation: $allocation,
            })
        }
    };
}

impl<'de, D: Deserializer<'de>> Deserializer<'de> for Guard<'_, D> {
    type Error = D::Error;
    forward_deserialize!(deserialize_any; false);
    forward_deserialize!(deserialize_bool; false);
    forward_deserialize!(deserialize_i8; false);
    forward_deserialize!(deserialize_i16; false);
    forward_deserialize!(deserialize_i32; false);
    forward_deserialize!(deserialize_i64; false);
    forward_deserialize!(deserialize_i128; false);
    forward_deserialize!(deserialize_u8; false);
    forward_deserialize!(deserialize_u16; false);
    forward_deserialize!(deserialize_u32; false);
    forward_deserialize!(deserialize_u64; false);
    forward_deserialize!(deserialize_u128; false);
    forward_deserialize!(deserialize_f32; false);
    forward_deserialize!(deserialize_f64; false);
    forward_deserialize!(deserialize_char; false);
    forward_deserialize!(deserialize_str; false);

    fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        // Bincode's owned path allocates from the declared length before
        // visiting. Borrow from the frame instead, charge the shared budget,
        // then create the owned value and preserve the requested callback.
        self.inner.deserialize_str(Guard {
            inner: Own(visitor),
            budget: self.budget,
            allocation: false,
        })
    }

    forward_deserialize!(deserialize_bytes; false);

    fn deserialize_byte_buf<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.inner.deserialize_bytes(Guard {
            inner: Own(visitor),
            budget: self.budget,
            allocation: false,
        })
    }

    forward_deserialize!(deserialize_option; false);
    forward_deserialize!(deserialize_unit; false);
    forward_deserialize!(deserialize_seq; true);
    forward_deserialize!(deserialize_map; true);
    forward_deserialize!(deserialize_identifier; false);
    forward_deserialize!(deserialize_ignored_any; false);
    forward_deserialize!(deserialize_unit_struct, name: &'static str; false);
    forward_deserialize!(deserialize_newtype_struct, name: &'static str; false);
    forward_deserialize!(deserialize_tuple, len: usize; false);
    forward_deserialize!(deserialize_tuple_struct, name: &'static str, len: usize; false);
    forward_deserialize!(deserialize_struct, name: &'static str, fields: &'static [&'static str]; false);
    forward_deserialize!(deserialize_enum, name: &'static str, variants: &'static [&'static str]; false);

    fn is_human_readable(&self) -> bool {
        self.inner.is_human_readable()
    }
}

// Guard checks borrowed contents before this adapter allocates. Keep owned
// callbacks for visitors that implement only visit_string or visit_byte_buf.
struct Own<V>(V);

impl<'de, V: Visitor<'de>> Visitor<'de> for Own<V> {
    type Value = V::Value;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.expecting(f)
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        self.0.visit_string(value.to_owned())
    }

    fn visit_bytes<E: de::Error>(self, value: &[u8]) -> Result<Self::Value, E> {
        self.0.visit_byte_buf(value.to_owned())
    }
}

macro_rules! forward_visit {
    ($method:ident, $ty:ty) => {
        fn $method<E: de::Error>(self, value: $ty) -> Result<Self::Value, E> {
            self.inner.$method(value)
        }
    };
}
impl<'de, V: Visitor<'de>> Visitor<'de> for Guard<'_, V> {
    type Value = V::Value;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.expecting(f)
    }

    forward_visit!(visit_bool, bool);
    forward_visit!(visit_i8, i8);
    forward_visit!(visit_i16, i16);
    forward_visit!(visit_i32, i32);
    forward_visit!(visit_i64, i64);
    forward_visit!(visit_i128, i128);
    forward_visit!(visit_u8, u8);
    forward_visit!(visit_u16, u16);
    forward_visit!(visit_u32, u32);
    forward_visit!(visit_u64, u64);
    forward_visit!(visit_u128, u128);
    forward_visit!(visit_f32, f32);
    forward_visit!(visit_f64, f64);
    forward_visit!(visit_char, char);

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        charge::<E>(self.budget, value.len())?;
        self.inner.visit_str(value)
    }

    fn visit_borrowed_str<E: de::Error>(self, value: &'de str) -> Result<Self::Value, E> {
        charge::<E>(self.budget, value.len())?;
        self.inner.visit_borrowed_str(value)
    }

    fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
        charge::<E>(self.budget, value.len())?;
        self.inner.visit_string(value)
    }

    fn visit_bytes<E: de::Error>(self, value: &[u8]) -> Result<Self::Value, E> {
        charge::<E>(self.budget, value.len())?;
        self.inner.visit_bytes(value)
    }

    fn visit_borrowed_bytes<E: de::Error>(self, value: &'de [u8]) -> Result<Self::Value, E> {
        charge::<E>(self.budget, value.len())?;
        self.inner.visit_borrowed_bytes(value)
    }

    fn visit_byte_buf<E: de::Error>(self, value: Vec<u8>) -> Result<Self::Value, E> {
        charge::<E>(self.budget, value.len())?;
        self.inner.visit_byte_buf(value)
    }

    fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
        self.inner.visit_none()
    }

    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        self.inner.visit_unit()
    }

    fn visit_some<D: Deserializer<'de>>(self, inner: D) -> Result<Self::Value, D::Error> {
        self.inner.visit_some(Guard {
            inner,
            budget: self.budget,
            allocation: false,
        })
    }

    fn visit_newtype_struct<D: Deserializer<'de>>(self, inner: D) -> Result<Self::Value, D::Error> {
        self.inner.visit_newtype_struct(Guard {
            inner,
            budget: self.budget,
            allocation: false,
        })
    }

    fn visit_seq<A: SeqAccess<'de>>(self, inner: A) -> Result<Self::Value, A::Error> {
        if self.allocation && inner.size_hint().is_some_and(|n| n > self.budget.get()) {
            return Err(de::Error::custom("collection length exceeds frame budget"));
        }
        self.inner.visit_seq(Guard {
            inner,
            budget: self.budget,
            allocation: self.allocation,
        })
    }

    fn visit_map<A: MapAccess<'de>>(self, inner: A) -> Result<Self::Value, A::Error> {
        if self.allocation && inner.size_hint().is_some_and(|n| n > self.budget.get()) {
            return Err(de::Error::custom("collection length exceeds frame budget"));
        }
        self.inner.visit_map(Guard {
            inner,
            budget: self.budget,
            allocation: self.allocation,
        })
    }

    fn visit_enum<A: EnumAccess<'de>>(self, inner: A) -> Result<Self::Value, A::Error> {
        self.inner.visit_enum(Guard {
            inner,
            budget: self.budget,
            allocation: false,
        })
    }
}

impl<'de, A: SeqAccess<'de>> SeqAccess<'de> for Guard<'_, A> {
    type Error = A::Error;

    fn next_element_seed<S: DeserializeSeed<'de>>(
        &mut self,
        seed: S,
    ) -> Result<Option<S::Value>, Self::Error> {
        if self.allocation && self.inner.size_hint() != Some(0) {
            charge::<Self::Error>(self.budget, mem::size_of::<S::Value>().max(1))?;
        }
        self.inner.next_element_seed(Seed {
            seed,
            budget: self.budget,
        })
    }

    fn size_hint(&self) -> Option<usize> {
        if self.allocation {
            Some(0)
        } else {
            self.inner.size_hint()
        }
    }
}

impl<'de, A: MapAccess<'de>> MapAccess<'de> for Guard<'_, A> {
    type Error = A::Error;

    fn next_key_seed<S: DeserializeSeed<'de>>(
        &mut self,
        seed: S,
    ) -> Result<Option<S::Value>, Self::Error> {
        if self.allocation && self.inner.size_hint() != Some(0) {
            charge::<Self::Error>(self.budget, mem::size_of::<S::Value>().max(1))?;
        }
        self.inner.next_key_seed(Seed {
            seed,
            budget: self.budget,
        })
    }

    fn next_value_seed<S: DeserializeSeed<'de>>(
        &mut self,
        seed: S,
    ) -> Result<S::Value, Self::Error> {
        if self.allocation {
            charge::<Self::Error>(self.budget, mem::size_of::<S::Value>().max(1))?;
        }
        self.inner.next_value_seed(Seed {
            seed,
            budget: self.budget,
        })
    }

    fn size_hint(&self) -> Option<usize> {
        if self.allocation {
            Some(0)
        } else {
            self.inner.size_hint()
        }
    }
}

impl<'a, 'de, A: EnumAccess<'de>> EnumAccess<'de> for Guard<'a, A> {
    type Error = A::Error;
    type Variant = Guard<'a, A::Variant>;

    fn variant_seed<S: DeserializeSeed<'de>>(
        self,
        seed: S,
    ) -> Result<(S::Value, Self::Variant), Self::Error> {
        let (value, inner) = self.inner.variant_seed(Seed {
            seed,
            budget: self.budget,
        })?;
        Ok((
            value,
            Guard {
                inner,
                budget: self.budget,
                allocation: false,
            },
        ))
    }
}

impl<'de, A: VariantAccess<'de>> VariantAccess<'de> for Guard<'_, A> {
    type Error = A::Error;

    fn unit_variant(self) -> Result<(), Self::Error> {
        self.inner.unit_variant()
    }

    fn newtype_variant_seed<S: DeserializeSeed<'de>>(
        self,
        seed: S,
    ) -> Result<S::Value, Self::Error> {
        self.inner.newtype_variant_seed(Seed {
            seed,
            budget: self.budget,
        })
    }

    fn tuple_variant<V: Visitor<'de>>(
        self,
        len: usize,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.inner.tuple_variant(
            len,
            Guard {
                inner: visitor,
                budget: self.budget,
                allocation: false,
            },
        )
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.inner.struct_variant(
            fields,
            Guard {
                inner: visitor,
                budget: self.budget,
                allocation: false,
            },
        )
    }
}
