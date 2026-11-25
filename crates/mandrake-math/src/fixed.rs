//! Complete 16.16 fixed-point arithmetic (`Fixed16`).
//!
//! Provides deterministic signed 32-bit fixed-point numbers with 16-bit integer
//! and 16-bit fractional parts (Q16.16 format).

use core::cmp::Ordering;
use core::fmt;
use core::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Sub, SubAssign};

/// Integer square root of a 64-bit unsigned integer, rounded to the nearest integer.
#[inline]
pub const fn isqrt64_round(val: u64) -> u64 {
    if val == 0 {
        return 0;
    }
    let mut rem = val;
    let mut root = 0u64;
    let mut bit = 1u64 << 62;

    while bit > rem {
        bit >>= 2;
    }

    while bit != 0 {
        if rem >= root + bit {
            rem -= root + bit;
            root = (root >> 1) + bit;
        } else {
            root >>= 1;
        }
        bit >>= 2;
    }

    if rem > root {
        root += 1;
    }

    root
}

/// 32-bit signed fixed-point number with 16 fractional bits (16.16).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize,
)]
#[repr(transparent)]
pub struct Fixed16(pub i32);

impl Fixed16 {
    pub const ZERO: Self = Self(0);
    pub const ONE: Self = Self(1 << 16);
    pub const HALF: Self = Self(1 << 15);
    pub const FRAC_BITS: u32 = 16;
    pub const SCALE: i32 = 1 << Self::FRAC_BITS;

    #[inline]
    pub const fn from_raw(raw: i32) -> Self {
        Self(raw)
    }

    #[inline]
    pub const fn to_raw(self) -> i32 {
        self.0
    }

    #[inline]
    pub const fn from_i32(val: i32) -> Self {
        Self(val << 16)
    }

    /// Truncates toward zero.
    #[inline]
    pub const fn to_i32(self) -> i32 {
        self.0 / Self::SCALE
    }

    /// Rounds to nearest 1/65536 step.
    #[inline]
    pub fn from_f32(val: f32) -> Self {
        let scaled = val * 65536.0;
        let rounded = if scaled >= 0.0 {
            (scaled + 0.5) as i32
        } else {
            (scaled - 0.5) as i32
        };
        Self(rounded)
    }

    #[inline]
    pub fn to_f32(self) -> f32 {
        self.0 as f32 / 65536.0
    }
}

impl PartialOrd for Fixed16 {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Fixed16 {
    #[inline]
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}

impl fmt::Display for Fixed16 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.4}", self.to_f32())
    }
}

macro_rules! forward_ref_binop {
    ($trait:ident, $method:ident, $lhs:ty, $rhs:ty, $out:ty) => {
        impl $trait<&$rhs> for $lhs {
            type Output = $out;
            #[inline]
            fn $method(self, rhs: &$rhs) -> $out {
                $trait::$method(self, *rhs)
            }
        }
        impl $trait<$rhs> for &$lhs {
            type Output = $out;
            #[inline]
            fn $method(self, rhs: $rhs) -> $out {
                $trait::$method(*self, rhs)
            }
        }
        impl $trait<&$rhs> for &$lhs {
            type Output = $out;
            #[inline]
            fn $method(self, rhs: &$rhs) -> $out {
                $trait::$method(*self, *rhs)
            }
        }
    };
}

macro_rules! forward_ref_assign {
    ($trait:ident, $method:ident, $lhs:ty, $rhs:ty) => {
        impl $trait<&$rhs> for $lhs {
            #[inline]
            fn $method(&mut self, rhs: &$rhs) {
                $trait::$method(self, *rhs);
            }
        }
    };
}

pub(crate) use {forward_ref_assign, forward_ref_binop};

impl Add for Fixed16 {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self {
        Self(self.0.wrapping_add(rhs.0))
    }
}
forward_ref_binop!(Add, add, Fixed16, Fixed16, Fixed16);

impl AddAssign for Fixed16 {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        self.0 = self.0.wrapping_add(rhs.0);
    }
}
forward_ref_assign!(AddAssign, add_assign, Fixed16, Fixed16);

impl Sub for Fixed16 {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self {
        Self(self.0.wrapping_sub(rhs.0))
    }
}
forward_ref_binop!(Sub, sub, Fixed16, Fixed16, Fixed16);

impl SubAssign for Fixed16 {
    #[inline]
    fn sub_assign(&mut self, rhs: Self) {
        self.0 = self.0.wrapping_sub(rhs.0);
    }
}
forward_ref_assign!(SubAssign, sub_assign, Fixed16, Fixed16);

impl Neg for Fixed16 {
    type Output = Self;
    #[inline]
    fn neg(self) -> Self {
        Self(self.0.wrapping_neg())
    }
}

impl Neg for &Fixed16 {
    type Output = Fixed16;
    #[inline]
    fn neg(self) -> Fixed16 {
        -(*self)
    }
}

impl Mul for Fixed16 {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: Self) -> Self {
        Self(((self.0 as i64 * rhs.0 as i64) >> 16) as i32)
    }
}
forward_ref_binop!(Mul, mul, Fixed16, Fixed16, Fixed16);

impl MulAssign for Fixed16 {
    #[inline]
    fn mul_assign(&mut self, rhs: Self) {
        *self = *self * rhs;
    }
}
forward_ref_assign!(MulAssign, mul_assign, Fixed16, Fixed16);

impl Div for Fixed16 {
    type Output = Self;
    #[inline]
    fn div(self, rhs: Self) -> Self {
        assert!(rhs.0 != 0, "attempt to divide Fixed16 by zero");
        Self((((self.0 as i64) << 16) / (rhs.0 as i64)) as i32)
    }
}
forward_ref_binop!(Div, div, Fixed16, Fixed16, Fixed16);

impl DivAssign for Fixed16 {
    #[inline]
    fn div_assign(&mut self, rhs: Self) {
        *self = *self / rhs;
    }
}
forward_ref_assign!(DivAssign, div_assign, Fixed16, Fixed16);

impl From<i32> for Fixed16 {
    #[inline]
    fn from(v: i32) -> Self {
        Self::from_i32(v)
    }
}

impl From<i16> for Fixed16 {
    #[inline]
    fn from(v: i16) -> Self {
        Self::from_i32(v as i32)
    }
}

impl From<i8> for Fixed16 {
    #[inline]
    fn from(v: i8) -> Self {
        Self::from_i32(v as i32)
    }
}

impl From<Fixed16> for f32 {
    #[inline]
    fn from(v: Fixed16) -> Self {
        v.to_f32()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants() {
        assert_eq!(Fixed16::ZERO.0, 0);
        assert_eq!(Fixed16::ONE.0, 65536);
        assert_eq!(Fixed16::HALF.0, 32768);
        assert_eq!(Fixed16::ONE.to_f32(), 1.0);
        assert_eq!(Fixed16::HALF.to_f32(), 0.5);
    }

    #[test]
    fn test_basic_arithmetic() {
        let a = Fixed16::from_i32(3);
        let b = Fixed16::from_i32(2);

        assert_eq!((a + b).to_i32(), 5);
        assert_eq!((a - b).to_i32(), 1);
        assert_eq!((a * b).to_i32(), 6);
        assert_eq!((-a).to_i32(), -3);

        let half = Fixed16::HALF;
        assert_eq!((a * half).to_f32(), 1.5);
        assert_eq!((a / b).to_f32(), 1.5);
    }

    #[test]
    fn wrapping_arithmetic_matches_32bit_twos_complement() {
        let max = Fixed16::from_raw(i32::MAX);
        let min = Fixed16::from_raw(i32::MIN);
        assert_eq!(max + Fixed16::from_raw(1), min);
        assert_eq!(min - Fixed16::from_raw(1), max);
        assert_eq!(-min, min);
    }

    #[test]
    #[should_panic(expected = "attempt to divide Fixed16 by zero")]
    fn test_division_by_zero_panics() {
        let a = Fixed16::ONE;
        let _ = a / Fixed16::ZERO;
    }
}
