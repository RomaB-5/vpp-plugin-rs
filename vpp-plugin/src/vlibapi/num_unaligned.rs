//! Utilities for unaligned numeric types used in VPP API messages.

use std::{
    cmp::Ordering,
    fmt,
    num::ParseIntError,
    ops::{Add, Div, Mul, Sub},
    str::FromStr,
};

macro_rules! unaligned_integer {
    (
        Self = $Ty:ident,
        Primitive = $Int:ident,

        // Used in doc comments.
        swap_op = $swap_op:literal,
        swapped = $swapped:literal,
    ) => {
        #[doc = concat!("A ", stringify!($Int), " that has an alignment requirement of 1 byte, i.e. is unaligned")]
        ///
        /// This is useful in packed structures and slices of data contained in variable-length
        /// arrays in VPP messages.
        ///
        /// # Layout
        ///
        #[doc = concat!("`", stringify!($Ty), "` is guaranteed to have the same layout and bit validity as `", stringify!($Int), "`.")]
        ///
        /// They are also guaranteed to have the same size.
        #[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[repr(C, packed)]
        pub struct $Ty($Int);

        impl $Ty {
            #[doc = concat!("Creates a ", stringify!($Int), " that has an alignment requirement of 1 byte, i.e. is unaligned.")]
            #[inline]
            pub const fn new(value: $Int) -> Self {
                Self(value)
            }

            /// Returns the contained value as a primitive type.
            #[inline]
            pub const fn get(self) -> $Int {
                self.0
            }

            /// Reverses the byte order of the integer.
            ///
            /// # Examples
            ///
            /// ```
            #[doc = concat!("# use vpp_plugin::vlibapi::num_unaligned::", stringify!($Ty), ";")]
            #[doc = concat!("let n = ", stringify!($Ty), "::new(", $swap_op, stringify!($Int), ");")]
            /// let m = n.swap_bytes();
            ///
            #[doc = concat!("assert_eq!(m, ", $swapped, ");")]
            /// ```
            #[must_use = "this returns the result of the operation, \
                        without modifying the original"]
            #[inline(always)]
            pub const fn swap_bytes(self) -> Self {
                Self(self.get().swap_bytes())
            }

            /// Converts `self` to big endian from the target's endianness.
            ///
            /// On big endian this is a no-op. On little endian the bytes are
            /// swapped.
            ///
            /// # Examples
            ///
            /// ```
            #[doc = concat!("# use vpp_plugin::vlibapi::num_unaligned::", stringify!($Ty), ";")]
            #[doc = concat!("let n = ", stringify!($Ty), "::new(0x1A", stringify!($Int), ");")]
            ///
            /// if cfg!(target_endian = "big") {
            ///     assert_eq!(n.to_be(), n)
            /// } else {
            ///     assert_eq!(n.to_be(), n.swap_bytes())
            /// }
            /// ```
            #[must_use = "this returns the result of the operation, \
                        without modifying the original"]
            #[inline]
            pub const fn to_be(self) -> Self {
                Self(self.get().to_be())
            }

            /// Converts an integer from big endian to the target's endianness.
            ///
            /// On big endian this is a no-op. On little endian the bytes are
            /// swapped.
            ///
            /// # Examples
            ///
            /// ```
            #[doc = concat!("# use vpp_plugin::vlibapi::num_unaligned::", stringify!($Ty), ";")]
            #[doc = concat!("let n = ", stringify!($Ty), "::new(0x1A", stringify!($Int), ");")]
            ///
            /// if cfg!(target_endian = "big") {
            #[doc = concat!("    assert_eq!(", stringify!($Ty), "::from_be(n), n)")]
            /// } else {
            #[doc = concat!("    assert_eq!(", stringify!($Ty), "::from_be(n), n.swap_bytes())")]
            /// }
            /// ```
            #[must_use = "this returns the result of the operation, \
                        without modifying the original"]
            #[inline(always)]
            pub const fn from_be(x: Self) -> Self {
                Self::new($Int::from_be(x.get()))
            }
        }

        impl fmt::Debug for $Ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.get().fmt(f)
            }
        }

        impl fmt::Display for $Ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.get().fmt(f)
            }
        }

        impl From<$Int> for $Ty {
            #[inline]
            fn from(value: $Int) -> Self {
                Self::new(value)
            }
        }

        impl From<$Ty> for $Int {
            #[inline]
            fn from(value: $Ty) -> Self {
                value.get()
            }
        }

        impl PartialEq<$Int> for $Ty {
            #[inline]
            fn eq(&self, other: &$Int) -> bool {
                self.get() == *other
            }
        }

        impl PartialOrd<$Int> for $Ty {
            #[inline]
            fn partial_cmp(&self, other: &$Int) -> Option<Ordering> {
                self.get().partial_cmp(&other)
            }
        }

        impl FromStr for $Ty {
            type Err = ParseIntError;

            #[inline]
            fn from_str(src: &str) -> Result<Self, Self::Err> {
                Ok(Self::new(src.parse::<$Int>()?))
            }
        }

        impl Add for $Ty {
            type Output = Self;

            #[inline]
            fn add(self, rhs: Self) -> Self {
                Self::new(self.get() + rhs.get())
            }
        }

        impl Add<$Int> for $Ty {
            type Output = $Int;

            #[inline]
            fn add(self, rhs: $Int) -> $Int {
                self.get() + rhs
            }
        }

        impl Sub for $Ty {
            type Output = Self;

            #[inline]
            fn sub(self, rhs: Self) -> Self {
                Self::new(self.get() - rhs.get())
            }
        }

        impl Sub<$Int> for $Ty {
            type Output = $Int;

            #[inline]
            fn sub(self, rhs: $Int) -> $Int {
                self.get() - rhs
            }
        }

        impl Mul for $Ty {
            type Output = Self;

            #[inline]
            fn mul(self, rhs: Self) -> Self {
                Self::new(self.get() * rhs.get())
            }
        }

        impl Mul<$Int> for $Ty {
            type Output = $Int;

            #[inline]
            fn mul(self, rhs: $Int) -> $Int {
                self.get() * rhs
            }
        }

        impl Div for $Ty {
            type Output = Self;

            #[inline]
            fn div(self, rhs: Self) -> Self {
                Self::new(self.get() / rhs.get())
            }
        }

        impl Div<$Int> for $Ty {
            type Output = $Int;

            #[inline]
            fn div(self, rhs: $Int) -> $Int {
                self.get() / rhs
            }
        }
    }
}

unaligned_integer! {
    Self = UnalignedU16,
    Primitive = u16,
    swap_op = "0x1234",
    swapped = "0x3412",
}

unaligned_integer! {
    Self = UnalignedI16,
    Primitive = i16,
    swap_op = "0x1234",
    swapped = "0x3412",
}

unaligned_integer! {
    Self = UnalignedU32,
    Primitive = u32,
    swap_op = "0x12345678",
    swapped = "0x78563412",
}

unaligned_integer! {
    Self = UnalignedI32,
    Primitive = i32,
    swap_op = "0x12345678",
    swapped = "0x78563412",
}

unaligned_integer! {
    Self = UnalignedU64,
    Primitive = u64,
    swap_op = "0x1234567890123456",
    swapped = "0x5634129078563412",
}

unaligned_integer! {
    Self = UnalignedI64,
    Primitive = i64,
    swap_op = "0x1234567890123456",
    swapped = "0x5634129078563412",
}

/// A f64 that has an alignment requirement of 1 byte, i.e. is unaligned
///
/// This is useful in packed structures and slices of data contained in variable-length
/// arrays in VPP messages.
///
/// # Layout
///
/// `UnalignedF64` is guaranteed to have the same layout and bit validity as `f64`.
///
/// They are also guaranteed to have the same size.
#[derive(Copy, Clone, PartialEq, PartialOrd)]
#[repr(C, packed)]
pub struct UnalignedF64(f64);

impl UnalignedF64 {
    /// Creates a f64 that has an alignment requirement of 1 byte, i.e. is unaligned.
    #[inline]
    pub const fn new(value: f64) -> Self {
        Self(value)
    }

    /// Returns the contained value as a primitive type.
    #[inline]
    pub const fn get(self) -> f64 {
        self.0
    }

    /// Reverses the byte order of the float.
    ///
    /// # Examples
    ///
    /// ```
    /// # use vpp_plugin::vlibapi::num_unaligned::UnalignedF64;
    /// let n = UnalignedF64::new(1.0f64);
    /// let m = n.swap_bytes();
    /// // The result depends on the endianness of the target
    /// ```
    #[must_use = "this returns the result of the operation, \
                without modifying the original"]
    #[inline(always)]
    pub fn swap_bytes(self) -> Self {
        let bits = self.0.to_bits().swap_bytes();
        Self(f64::from_bits(bits))
    }

    /// Converts `self` to big endian from the target's endianness.
    ///
    /// On big endian this is a no-op. On little endian the bytes are
    /// swapped.
    ///
    /// # Examples
    ///
    /// ```
    /// # use vpp_plugin::vlibapi::num_unaligned::UnalignedF64;
    /// let n = UnalignedF64::new(1.0f64);
    ///
    /// if cfg!(target_endian = "big") {
    ///     assert_eq!(n.to_be(), n)
    /// } else {
    ///     assert_eq!(n.to_be(), n.swap_bytes())
    /// }
    /// ```
    #[must_use = "this returns the result of the operation, \
                without modifying the original"]
    #[inline]
    pub fn to_be(self) -> Self {
        let bits = self.0.to_bits().to_be();
        Self(f64::from_bits(bits))
    }

    /// Converts a float from big endian to the target's endianness.
    ///
    /// On big endian this is a no-op. On little endian the bytes are
    /// swapped.
    ///
    /// # Examples
    ///
    /// ```
    /// # use vpp_plugin::vlibapi::num_unaligned::UnalignedF64;
    /// let n = UnalignedF64::new(1.0f64);
    ///
    /// if cfg!(target_endian = "big") {
    ///     assert_eq!(UnalignedF64::from_be(n), n)
    /// } else {
    ///     assert_eq!(UnalignedF64::from_be(n), n.swap_bytes())
    /// }
    /// ```
    #[must_use = "this returns the result of the operation, \
                without modifying the original"]
    #[inline(always)]
    pub fn from_be(x: Self) -> Self {
        let bits = u64::from_be(x.0.to_bits());
        Self(f64::from_bits(bits))
    }
}

impl fmt::Debug for UnalignedF64 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(f)
    }
}

impl fmt::Display for UnalignedF64 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(f)
    }
}

impl From<f64> for UnalignedF64 {
    #[inline]
    fn from(value: f64) -> Self {
        Self::new(value)
    }
}

impl From<UnalignedF64> for f64 {
    #[inline]
    fn from(value: UnalignedF64) -> Self {
        value.get()
    }
}

impl PartialEq<f64> for UnalignedF64 {
    #[inline]
    fn eq(&self, other: &f64) -> bool {
        self.get() == *other
    }
}

impl PartialOrd<f64> for UnalignedF64 {
    #[inline]
    fn partial_cmp(&self, other: &f64) -> Option<Ordering> {
        self.get().partial_cmp(other)
    }
}

impl FromStr for UnalignedF64 {
    type Err = <f64 as FromStr>::Err;

    #[inline]
    fn from_str(src: &str) -> Result<Self, Self::Err> {
        Ok(Self::new(src.parse::<f64>()?))
    }
}

impl Add for UnalignedF64 {
    type Output = Self;

    #[inline]
    fn add(self, rhs: Self) -> Self {
        Self::new(self.get() + rhs.get())
    }
}

impl Add<f64> for UnalignedF64 {
    type Output = f64;

    #[inline]
    fn add(self, rhs: f64) -> f64 {
        self.get() + rhs
    }
}

impl Sub for UnalignedF64 {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.get() - rhs.get())
    }
}

impl Sub<f64> for UnalignedF64 {
    type Output = f64;

    #[inline]
    fn sub(self, rhs: f64) -> f64 {
        self.get() - rhs
    }
}

impl Mul for UnalignedF64 {
    type Output = Self;

    #[inline]
    fn mul(self, rhs: Self) -> Self {
        Self::new(self.get() * rhs.get())
    }
}

impl Mul<f64> for UnalignedF64 {
    type Output = f64;

    #[inline]
    fn mul(self, rhs: f64) -> f64 {
        self.get() * rhs
    }
}

impl Div for UnalignedF64 {
    type Output = Self;

    #[inline]
    fn div(self, rhs: Self) -> Self {
        Self::new(self.get() / rhs.get())
    }
}

impl Div<f64> for UnalignedF64 {
    type Output = f64;

    #[inline]
    fn div(self, rhs: f64) -> f64 {
        self.get() / rhs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! unaligned_numeric_tests {
        (
            Self = $Ty:ident,
            Primitive = $Prim:ident,
            swap_value = $swap_val:expr,
            test_value = $test_val:expr,
            test_value_str = $test_val_str:literal,
            add_value = $add_val:expr,
            sub_value = $sub_val:expr,
            mul_value = $mul_val:expr,
            div_value = $div_val:expr,
        ) => {
            paste::paste! {
                #[test]
                fn [<test_ $Ty:snake _new_and_get>]() {
                    let value = $test_val;
                    let unaligned = $Ty::new(value);
                    assert_eq!(unaligned.get(), value);
                }

                #[test]
                fn [<test_ $Ty:snake _from_be>]() {
                    let value = $swap_val;
                    let unaligned = $Ty::new(value);
                    let be = unaligned.to_be();
                    let from_be = $Ty::from_be(be);
                    assert_eq!(from_be, unaligned);
                }

                #[test]
                fn [<test_ $Ty:snake _from_ $Prim:snake>]() {
                    let value = $test_val;
                    let unaligned: $Ty = value.into();
                    assert_eq!(unaligned.get(), value);
                }

                #[test]
                fn [<test_ $Ty:snake _into_ $Prim:snake>]() {
                    let value = $test_val;
                    let unaligned = $Ty::new(value);
                    let primitive: $Prim = unaligned.into();
                    assert_eq!(primitive, value);
                }

                #[test]
                fn [<test_ $Ty:snake _partial_eq_ $Prim:snake>]() {
                    let unaligned = $Ty::new($test_val);
                    assert_eq!(unaligned, $test_val);
                    assert_ne!(unaligned, $test_val + $add_val);
                }

                #[test]
                fn [<test_ $Ty:snake _partial_ord_ $Prim:snake>]() {
                    let unaligned = $Ty::new($test_val);
                    assert!(unaligned < $test_val + $add_val);
                    assert!(unaligned > $test_val - $add_val);
                    assert!(unaligned <= $test_val);
                    assert!(unaligned >= $test_val);
                }

                #[test]
                fn [<test_ $Ty:snake _from_str>]() {
                    let unaligned: $Ty = $test_val_str.parse().unwrap();
                    assert_eq!(unaligned.get(), $test_val);
                    assert_eq!(unaligned.to_string(), $test_val_str);
                }

                #[test]
                fn [<test_ $Ty:snake _from_str_invalid>]() {
                    let result: Result<$Ty, _> = "invalid".parse();
                    assert!(result.is_err());
                }

                #[test]
                fn [<test_ $Ty:snake _add>]() {
                    let a = $Ty::new($test_val);
                    let b = $Ty::new($add_val);
                    let result = a + b;
                    assert_eq!(result.get(), $test_val + $add_val);
                }

                #[test]
                fn [<test_ $Ty:snake _add_ $Prim:snake>]() {
                    let unaligned = $Ty::new($test_val);
                    let result = unaligned + $add_val;
                    assert_eq!(result, $test_val + $add_val);
                }

                #[test]
                fn [<test_ $Ty:snake _sub>]() {
                    let a = $Ty::new($test_val + $sub_val);
                    let b = $Ty::new($sub_val);
                    let result = a - b;
                    assert_eq!(result.get(), $test_val);
                }

                #[test]
                fn [<test_ $Ty:snake _sub_ $Prim:snake>]() {
                    let unaligned = $Ty::new($test_val + $sub_val);
                    let result = unaligned - $sub_val;
                    assert_eq!(result, $test_val);
                }

                #[test]
                fn [<test_ $Ty:snake _mul>]() {
                    let a = $Ty::new($test_val);
                    let b = $Ty::new($mul_val);
                    let result = a * b;
                    assert_eq!(result.get(), $test_val * $mul_val);
                }

                #[test]
                fn [<test_ $Ty:snake _mul_ $Prim:snake>]() {
                    let unaligned = $Ty::new($test_val);
                    let result = unaligned * $mul_val;
                    assert_eq!(result, $test_val * $mul_val);
                }

                #[test]
                fn [<test_ $Ty:snake _div>]() {
                    let a = $Ty::new($test_val * $div_val);
                    let b = $Ty::new($div_val);
                    let result = a / b;
                    assert_eq!(result.get(), $test_val);
                }

                #[test]
                fn [<test_ $Ty:snake _div_ $Prim:snake>]() {
                    let unaligned = $Ty::new($test_val * $div_val);
                    let result = unaligned / $div_val;
                    assert_eq!(result, $test_val);
                }

                #[test]
                fn [<test_ $Ty:snake _debug>]() {
                    let unaligned = $Ty::new($test_val);
                    assert_eq!(format!("{:?}", unaligned), $test_val_str);
                }

                #[test]
                fn [<test_ $Ty:snake _clone>]() {
                    let original = $Ty::new($test_val);
                    let cloned = original.clone();
                    assert_eq!(original, cloned);
                    assert_eq!(original.get(), cloned.get());
                }

                #[test]
                fn [<test_ $Ty:snake _copy>]() {
                    let original = $Ty::new($test_val);
                    let copied = original;
                    assert_eq!(original, copied);
                    assert_eq!(original.get(), copied.get());
                }

                #[test]
                fn [<test_ $Ty:snake _min>]() {
                    let min = $Ty::new($Prim::MIN);
                    assert_eq!(min.get(), $Prim::MIN);
                    assert_eq!(min, $Prim::MIN);
                }

                #[test]
                fn [<test_ $Ty:snake _max>]() {
                    let max = $Ty::new($Prim::MAX);
                    assert_eq!(max.get(), $Prim::MAX);
                    assert_eq!(max, $Prim::MAX);
                }
            }
        };
    }

    macro_rules! unaligned_integer_tests {
        (
            Self = $Ty:ident,
            Primitive = $Prim:ident,
            swap_value = $swap_val:expr,
            test_value = $test_val:expr,
            test_value_str = $test_val_str:literal,
            add_value = $add_val:expr,
            sub_value = $sub_val:expr,
            mul_value = $mul_val:expr,
            div_value = $div_val:expr,
        ) => {
            unaligned_numeric_tests! {
                Self = $Ty,
                Primitive = $Prim,
                swap_value = $swap_val,
                test_value = $test_val,
                test_value_str = $test_val_str,
                add_value = $add_val,
                sub_value = $sub_val,
                mul_value = $mul_val,
                div_value = $div_val,
            }

            paste::paste! {
                #[test]
                fn [<test_ $Ty:snake _swap_bytes>]() {
                    let value = $swap_val;
                    let unaligned = $Ty::new(value);
                    let swapped = unaligned.swap_bytes();
                    assert_eq!(swapped.get(), value.swap_bytes());
                }

                #[test]
                fn [<test_ $Ty:snake _to_be>]() {
                    let value = $swap_val;
                    let unaligned = $Ty::new(value);
                    let be = unaligned.to_be();
                    assert_eq!(be.get(), value.to_be());
                }

                #[test]
                fn [<test_ $Ty:snake _hash>]() {
                    use std::collections::HashSet;
                    let mut set = HashSet::new();
                    let a = $Ty::new(1);
                    let b = $Ty::new(1);
                    set.insert(a);
                    assert!(set.contains(&b));
                }

                #[test]
                fn [<test_ $Ty:snake _ord>]() {
                    let a = $Ty::new(1);
                    let b = $Ty::new(2);
                    let c = $Ty::new(1);
                    assert!(a < b);
                    assert!(a <= c);
                    assert!(b > a);
                    assert!(a >= c);
                    assert_eq!(a.cmp(&c), std::cmp::Ordering::Equal);
                }
            }
        };
    }

    unaligned_integer_tests! {
        Self = UnalignedU16,
        Primitive = u16,
        swap_value = 0x1234u16,
        test_value = 42u16,
        test_value_str = "42",
        add_value = 20u16,
        sub_value = 10u16,
        mul_value = 7u16,
        div_value = 5u16,
    }

    unaligned_integer_tests! {
        Self = UnalignedI16,
        Primitive = i16,
        swap_value = 0x1234i16,
        test_value = 42i16,
        test_value_str = "42",
        add_value = 20i16,
        sub_value = 10i16,
        mul_value = 7i16,
        div_value = 5i16,
    }

    unaligned_integer_tests! {
        Self = UnalignedU32,
        Primitive = u32,
        swap_value = 0x12345678u32,
        test_value = 42u32,
        test_value_str = "42",
        add_value = 20u32,
        sub_value = 10u32,
        mul_value = 7u32,
        div_value = 5u32,
    }

    unaligned_integer_tests! {
        Self = UnalignedI32,
        Primitive = i32,
        swap_value = 0x12345678i32,
        test_value = 42i32,
        test_value_str = "42",
        add_value = 20i32,
        sub_value = 10i32,
        mul_value = 7i32,
        div_value = 5i32,
    }

    unaligned_integer_tests! {
        Self = UnalignedU64,
        Primitive = u64,
        swap_value = 0x1234567890123456u64,
        test_value = 42u64,
        test_value_str = "42",
        add_value = 20u64,
        sub_value = 10u64,
        mul_value = 7u64,
        div_value = 5u64,
    }

    unaligned_integer_tests! {
        Self = UnalignedI64,
        Primitive = i64,
        swap_value = 0x1234567890123456i64,
        test_value = 42i64,
        test_value_str = "42",
        add_value = 20i64,
        sub_value = 10i64,
        mul_value = 7i64,
        div_value = 5i64,
    }

    unaligned_numeric_tests! {
        Self = UnalignedF64,
        Primitive = f64,
        swap_value = 1.0f64,
        test_value = 42.5f64,
        test_value_str = "42.5",
        add_value = 20.0f64,
        sub_value = 10.0f64,
        mul_value = 7.0f64,
        div_value = 5.0f64,
    }
    #[test]
    fn test_unaligned_f64_swap_bytes() {
        let value = 1.0f64;
        let unaligned = UnalignedF64::new(value);
        let swapped = unaligned.swap_bytes();
        assert_eq!(swapped.get().to_bits(), value.to_bits().swap_bytes());
    }

    #[test]
    fn test_unaligned_f64_to_be() {
        let value = 1.0f64;
        let unaligned = UnalignedF64::new(value);
        let be = unaligned.to_be();
        assert_eq!(be.get().to_bits(), value.to_bits().to_be());
    }
}
