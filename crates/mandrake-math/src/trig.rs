//! 512-step binary angle (`Angle512`) used by the `Unit` facing layer.
//! This is not the engine's 1024-step trigonometry.

/// Binary angle representation where 512 units equal 360 degrees (a full circle).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize,
)]
#[repr(transparent)]
pub struct Angle512(pub u16);

impl Angle512 {
    pub const ZERO: Self = Self(0);
    pub const COUNT: u16 = 512;
    pub const MASK: u16 = 511;

    #[inline]
    pub const fn from_raw(raw: u16) -> Self {
        Self(raw & Self::MASK)
    }

    #[inline]
    pub const fn to_raw(self) -> u16 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_raw_wraps_modulo_512() {
        assert_eq!(Angle512::from_raw(0).to_raw(), 0);
        assert_eq!(Angle512::from_raw(512).to_raw(), 0);
        assert_eq!(Angle512::from_raw(612).to_raw(), 100);
    }
}
