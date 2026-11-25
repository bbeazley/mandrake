//! 3D fixed-point vector math (`Vec3Fixed`).

use crate::fixed::{Fixed16, forward_ref_assign, forward_ref_binop};
use core::fmt;
use core::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Sub, SubAssign};

/// 3-dimensional vector with 16.16 fixed-point coordinates.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize,
)]
pub struct Vec3Fixed {
    pub x: Fixed16,
    pub y: Fixed16,
    pub z: Fixed16,
}

impl Vec3Fixed {
    pub const ZERO: Self = Self::new(Fixed16::ZERO, Fixed16::ZERO, Fixed16::ZERO);

    #[inline]
    pub const fn new(x: Fixed16, y: Fixed16, z: Fixed16) -> Self {
        Self { x, y, z }
    }
}

impl fmt::Display for Vec3Fixed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Vec3Fixed({}, {}, {})", self.x, self.y, self.z)
    }
}

impl Add for Vec3Fixed {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self {
        Self {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
            z: self.z + rhs.z,
        }
    }
}
forward_ref_binop!(Add, add, Vec3Fixed, Vec3Fixed, Vec3Fixed);

impl AddAssign for Vec3Fixed {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        self.x += rhs.x;
        self.y += rhs.y;
        self.z += rhs.z;
    }
}
forward_ref_assign!(AddAssign, add_assign, Vec3Fixed, Vec3Fixed);

impl Sub for Vec3Fixed {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self {
        Self {
            x: self.x - rhs.x,
            y: self.y - rhs.y,
            z: self.z - rhs.z,
        }
    }
}
forward_ref_binop!(Sub, sub, Vec3Fixed, Vec3Fixed, Vec3Fixed);

impl SubAssign for Vec3Fixed {
    #[inline]
    fn sub_assign(&mut self, rhs: Self) {
        self.x -= rhs.x;
        self.y -= rhs.y;
        self.z -= rhs.z;
    }
}
forward_ref_assign!(SubAssign, sub_assign, Vec3Fixed, Vec3Fixed);

impl Neg for Vec3Fixed {
    type Output = Self;
    #[inline]
    fn neg(self) -> Self {
        Self {
            x: -self.x,
            y: -self.y,
            z: -self.z,
        }
    }
}

impl Neg for &Vec3Fixed {
    type Output = Vec3Fixed;
    #[inline]
    fn neg(self) -> Vec3Fixed {
        -(*self)
    }
}

impl Mul<Fixed16> for Vec3Fixed {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: Fixed16) -> Self {
        Self {
            x: self.x * rhs,
            y: self.y * rhs,
            z: self.z * rhs,
        }
    }
}
forward_ref_binop!(Mul, mul, Vec3Fixed, Fixed16, Vec3Fixed);

impl Mul<Vec3Fixed> for Fixed16 {
    type Output = Vec3Fixed;
    #[inline]
    fn mul(self, rhs: Vec3Fixed) -> Vec3Fixed {
        rhs * self
    }
}
forward_ref_binop!(Mul, mul, Fixed16, Vec3Fixed, Vec3Fixed);

impl MulAssign<Fixed16> for Vec3Fixed {
    #[inline]
    fn mul_assign(&mut self, rhs: Fixed16) {
        self.x *= rhs;
        self.y *= rhs;
        self.z *= rhs;
    }
}
forward_ref_assign!(MulAssign, mul_assign, Vec3Fixed, Fixed16);

impl Div<Fixed16> for Vec3Fixed {
    type Output = Self;
    #[inline]
    fn div(self, rhs: Fixed16) -> Self {
        Self {
            x: self.x / rhs,
            y: self.y / rhs,
            z: self.z / rhs,
        }
    }
}
forward_ref_binop!(Div, div, Vec3Fixed, Fixed16, Vec3Fixed);

impl DivAssign<Fixed16> for Vec3Fixed {
    #[inline]
    fn div_assign(&mut self, rhs: Fixed16) {
        self.x /= rhs;
        self.y /= rhs;
        self.z /= rhs;
    }
}
forward_ref_assign!(DivAssign, div_assign, Vec3Fixed, Fixed16);

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: i32, y: i32, z: i32) -> Vec3Fixed {
        Vec3Fixed::new(
            Fixed16::from_i32(x),
            Fixed16::from_i32(y),
            Fixed16::from_i32(z),
        )
    }

    #[test]
    fn test_vec3_basic_ops() {
        let v1 = v(1, 2, 3);
        let v2 = v(4, 5, 6);

        assert_eq!(v1 + v2, v(5, 7, 9));
        assert_eq!(v2 - v1, v(3, 3, 3));
        assert_eq!(-v1, v(-1, -2, -3));
        assert_eq!(v1 * Fixed16::from_i32(2), v(2, 4, 6));
        assert_eq!(
            v2 / Fixed16::from_i32(2),
            Vec3Fixed::new(
                Fixed16::from_i32(2),
                Fixed16::from_f32(2.5),
                Fixed16::from_i32(3)
            )
        );
    }
}
