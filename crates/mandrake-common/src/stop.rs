//! Where a sim stops: one type for both games.

/// Where the port stops: the engine address of the site or branch it would need next, and why.
/// mandrake-sim calls it `Unmodelled`, mandrake-tfl-sim `Stop`.
///
/// The text is `0x<site>: <reason>`, which Myth II's film baseline splits into its `stop_addr`
/// and `stop` columns (`crates/mandrake-sim/tests/all/film_baseline.rs`, `stop_fields`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{site:#x}: {reason}")]
pub struct Stop {
    /// Engine address of the unimplemented site or branch.
    pub site: u32,
    pub reason: String,
}

impl Stop {
    pub fn new(site: u32, reason: impl Into<String>) -> Self {
        Self {
            site,
            reason: reason.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_the_site_in_hex_then_the_reason() {
        let stop = Stop::new(0x1234, "an unported branch");
        assert_eq!(stop.to_string(), "0x1234: an unported branch");
        assert_eq!(Stop::new(0, "x").to_string(), "0x0: x");
    }
}
