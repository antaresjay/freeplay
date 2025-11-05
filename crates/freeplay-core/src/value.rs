use std::fmt;
use std::str::FromStr;

macro_rules! define_scalars {
    ($($variant:ident($ty:ty) = $name:literal),* $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum ValueKind { $($variant,)* }

        #[derive(Debug, Clone, Copy, PartialEq)]
        pub enum Scalar { $($variant($ty),)* }

        impl ValueKind {
            pub const ALL: &'static [ValueKind] = &[$(ValueKind::$variant,)*];

            pub const fn size(self) -> usize {
                match self { $(ValueKind::$variant => std::mem::size_of::<$ty>(),)* }
            }

            pub const fn name(self) -> &'static str {
                match self { $(ValueKind::$variant => $name,)* }
            }

            pub fn read(self, bytes: &[u8]) -> Option<Scalar> {
                match self {
                    $(ValueKind::$variant => bytes
                        .get(..std::mem::size_of::<$ty>())
                        .and_then(|b| b.try_into().ok())
                        .map(|b| Scalar::$variant(<$ty>::from_ne_bytes(b))),)*
                }
            }

            pub fn parse(self, text: &str) -> Option<Scalar> {
                match self {
                    $(ValueKind::$variant => text.trim().parse::<$ty>().ok().map(Scalar::$variant),)*
                }
            }
        }

        impl Scalar {
            pub const fn kind(self) -> ValueKind {
                match self { $(Scalar::$variant(_) => ValueKind::$variant,)* }
            }

            pub fn to_bytes(self) -> Vec<u8> {
                match self { $(Scalar::$variant(v) => v.to_ne_bytes().to_vec(),)* }
            }

            pub fn as_f64(self) -> f64 {
                match self { $(Scalar::$variant(v) => v as f64,)* }
            }
        }

        impl fmt::Display for Scalar {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                match self { $(Scalar::$variant(v) => write!(f, "{v}"),)* }
            }
        }

        impl FromStr for ValueKind {
            type Err = String;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s.trim().to_ascii_lowercase().as_str() {
                    $($name => Ok(ValueKind::$variant),)*
                    other => Err(format!("unknown value type {other:?}")),
                }
            }
        }
    };
}

define_scalars! {
    I8(i8) = "i8",
    U8(u8) = "u8",
    I16(i16) = "i16",
    U16(u16) = "u16",
    I32(i32) = "i32",
    U32(u32) = "u32",
    I64(i64) = "i64",
    U64(u64) = "u64",
    F32(f32) = "f32",
    F64(f64) = "f64",
}

impl ValueKind {
    pub const fn is_float(self) -> bool {
        matches!(self, ValueKind::F32 | ValueKind::F64)
    }
}

impl Scalar {
    /// Floats are compared with a tolerance because a health bar that reads
    /// 100 on screen is rarely exactly 100.0 in memory, and a scan for the
    /// exact bit pattern finds nothing.
    pub fn matches(self, other: Scalar) -> bool {
        match (self, other) {
            (Scalar::F32(a), Scalar::F32(b)) => (a - b).abs() <= f32::EPSILON.max(a.abs() * 1e-4),
            (Scalar::F64(a), Scalar::F64(b)) => (a - b).abs() <= f64::EPSILON.max(a.abs() * 1e-6),
            _ => self == other,
        }
    }
}

impl fmt::Display for ValueKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_match_the_rust_types() {
        assert_eq!(ValueKind::I32.size(), 4);
        assert_eq!(ValueKind::F64.size(), 8);
        assert_eq!(ValueKind::U8.size(), 1);
    }

    #[test]
    fn roundtrips_through_bytes() {
        let value = Scalar::I32(-1234);
        let bytes = value.to_bytes();
        assert_eq!(ValueKind::I32.read(&bytes), Some(value));
    }

    #[test]
    fn short_buffer_reads_nothing() {
        assert_eq!(ValueKind::I64.read(&[0u8; 4]), None);
    }

    #[test]
    fn parses_type_names() {
        assert_eq!("f32".parse::<ValueKind>().unwrap(), ValueKind::F32);
        assert_eq!("U16".parse::<ValueKind>().unwrap(), ValueKind::U16);
        assert!("f128".parse::<ValueKind>().is_err());
    }

    #[test]
    fn floats_compare_with_tolerance() {
        assert!(Scalar::F32(100.0).matches(Scalar::F32(100.000_01)));
        assert!(!Scalar::F32(100.0).matches(Scalar::F32(101.0)));
    }

    #[test]
    fn integers_compare_exactly() {
        assert!(Scalar::I32(100).matches(Scalar::I32(100)));
        assert!(!Scalar::I32(100).matches(Scalar::I32(101)));
    }
}
