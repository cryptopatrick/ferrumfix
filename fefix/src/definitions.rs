//! Field and message definitions for all FIX application versions.

#[cfg(feature = "fix40")]
#[allow(dead_code)]
#[rustfmt::skip]
/// Field and message definitions for FIX.4.0.
pub mod fix40 {
    include!(concat!(env!("OUT_DIR"), "/fix40.rs"));
}

#[cfg(feature = "fix41")]
#[allow(dead_code)]
#[rustfmt::skip]
/// Field and message definitions for FIX.4.1.
pub mod fix41 {
    include!(concat!(env!("OUT_DIR"), "/fix41.rs"));
}

#[cfg(feature = "fix42")]
#[allow(dead_code)]
#[rustfmt::skip]
/// Field and message definitions for FIX.4.2.
pub mod fix42 {
    include!(concat!(env!("OUT_DIR"), "/fix42.rs"));
}

#[cfg(feature = "fix43")]
#[allow(dead_code)]
#[rustfmt::skip]
/// Field and message definitions for FIX.4.3.
pub mod fix43 {
    include!(concat!(env!("OUT_DIR"), "/fix43.rs"));
}

#[allow(dead_code)]
#[rustfmt::skip]
/// Field and message definitions for FIX.4.4.
pub mod fix44 {
    include!(concat!(env!("OUT_DIR"), "/fix44.rs"));
}

#[cfg(feature = "fix50")]
#[allow(dead_code)]
#[rustfmt::skip]
/// Field and message definitions for FIX.5.0.
pub mod fix50 {
    include!(concat!(env!("OUT_DIR"), "/fix50.rs"));
}

#[cfg(feature = "fix50sp1")]
#[allow(dead_code)]
#[rustfmt::skip]
/// Field and message definitions for FIX.5.0 SP1.
pub mod fix50sp1 {
    include!(concat!(env!("OUT_DIR"), "/fix50sp1.rs"));
}

#[cfg(feature = "fix50sp2")]
#[allow(dead_code)]
#[rustfmt::skip]
/// Field and message definitions for FIX.5.0 SP2.
pub mod fix50sp2 {
    include!(concat!(env!("OUT_DIR"), "/fix50sp2.rs"));
}

#[cfg(feature = "fixt11")]
#[allow(dead_code)]
#[rustfmt::skip]
/// Field and message definitions for FIXT.1.1.
pub mod fixt11 {
    include!(concat!(env!("OUT_DIR"), "/fixt11.rs"));
}

use crate::{dict, dict::FixDataType, FixFieldValue, TagU16};
use std::marker::PhantomData;

/// Metadata about a specific FIX tag. Designed for code generation.
///
/// # Type signature
///
/// `V` is the suggested [`FixFieldValue`] for this FIX field.
#[derive(Debug, Clone)]
pub struct GeneratedFieldDef<'a, V>
where
    V: FixFieldValue<'a>,
{
    /// Human-readable ASCII name of this FIX field, e.g. `MsgSeqNum`.
    pub name: &'static str,
    /// Numeric tag associated with this FIX field.
    pub tag: u16,
    pub is_group_leader: bool,
    pub data_type: FixDataType,
    /// Expected location of this FIX field within messages - header, body, or
    /// trailer. Used for JSON-encoded FIX messages.
    pub location: dict::FieldLocation,
    pub phantom: PhantomData<&'a V>,
}

impl<'a, V> dict::IsFieldDefinition for GeneratedFieldDef<'a, V>
where
    V: FixFieldValue<'a>,
{
    #[inline(always)]
    fn tag(&self) -> TagU16 {
        TagU16::new(self.tag).unwrap()
    }

    #[inline(always)]
    fn name(&self) -> &str {
        self.name
    }

    #[inline(always)]
    fn location(&self) -> dict::FieldLocation {
        self.location
    }
}

impl<'a, V> dict::IsTypedFieldDefinition<V> for GeneratedFieldDef<'a, V> where V: FixFieldValue<'a> {}