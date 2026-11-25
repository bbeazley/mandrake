//! `--water enhanced` wind ripples' wind: the game's wind (`EngineWind` velocity,
//! `get_wind_velocity`, read by the client) as the ripple wind the FFT cascades
//! (`mandrake_render::water::fft`), the fetch limit and the gusts are made for:
//! `U = ripple_calm + ripple_wind_scale * |wind|` metres per second, how much of it comes from
//! the game's wind (`windiness`, 0 isotropic in calm .. 1) and its direction in render space.

/// Myth II's ticks per second (`Simulation::TICKS_PER_SECOND`).
const TICKS_PER_SECOND: f64 = 30.0;

/// The ripple wind's settings and the last game wind it was made from.
#[derive(Clone, Debug)]
pub struct RippleWind {
    meters_per_cell: f32,
    calm: f32,
    wind_scale: f32,
    wind: Option<[i32; 3]>,
    logged: bool,
    /// The line [`Self::update`] made for the caller to log, and whether it was the first.
    log: Option<(bool, String)>,
}

impl RippleWind {
    /// `calm`: the ripple wind with no game wind (m/s); `wind_scale`: m/s of ripple wind per
    /// m/s of the game's.
    pub fn new(meters_per_cell: f32, calm: f32, wind_scale: f32) -> Self {
        Self {
            meters_per_cell,
            calm,
            wind_scale,
            wind: None,
            logged: false,
            log: None,
        }
    }

    /// The game's wind (16.16 world units per tick) as metres per second, and its direction in
    /// render space (x falls with the world x, z grows with the world y).
    fn game_wind(&self, wind: Option<[i32; 3]>) -> (f64, f64) {
        let Some(v) = wind else {
            return (0.0, 0.0);
        };
        let (x, y) = (f64::from(v[0]) / 65536.0, f64::from(v[1]) / 65536.0);
        let per_tick = (x * x + y * y).sqrt();
        let speed = per_tick * TICKS_PER_SECOND * f64::from(self.meters_per_cell)
            / f64::from(crate::WORLD_UNITS_PER_CELL);
        (speed, y.atan2(-x))
    }

    /// The ripple wind for the game's `wind`: speed (m/s), share from the game's wind (0..1)
    /// and direction (radians, render x/z). The first one and every change leave a line for
    /// [`Self::take_log`].
    pub fn update(&mut self, wind: Option<[i32; 3]>) -> (f64, f64, f64) {
        let (game, direction) = self.game_wind(wind);
        let u = f64::from(self.calm.max(0.0)) + f64::from(self.wind_scale.max(0.0)) * game;
        // Any game wind sets the direction (0.5 m/s and more: all of it).
        let windiness = {
            let t = (game / 0.5).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        if !self.logged || wind != self.wind {
            let first = !self.logged;
            self.logged = true;
            let line = format!(
                "--water enhanced wind ripples: game wind {:?} (16.16 world units per tick) = {game:.2} m/s toward {:.0} deg (render x/z), ripples for {u:.2} m/s, {:.0}% directional",
                wind,
                direction.to_degrees(),
                windiness * 100.0
            );
            self.log = Some((first, line));
            self.wind = wind;
        }
        (u, windiness, direction)
    }

    /// The line the last [`Self::update`] left to log (info for the first, debug after), once.
    pub fn take_log(&mut self) -> Option<(bool, String)> {
        self.log.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calm_is_isotropic_and_wind_sets_speed_and_direction() {
        let mut w = RippleWind::new(1.589, 1.5, 1.5);
        let (u, windiness, _) = w.update(None);
        assert_eq!((u, windiness), (1.5, 0.0));
        // 20 world units a tick along world +x: render direction 180 deg.
        let (u, windiness, direction) = w.update(Some([20 * 65536, 0, 0]));
        let game = 20.0 * TICKS_PER_SECOND * f64::from(1.589f32) / 512.0;
        assert!((u - (1.5 + 1.5 * game)).abs() < 1e-6, "{u}");
        assert_eq!(windiness, 1.0);
        assert!((direction.abs() - std::f64::consts::PI).abs() < 1e-9);
        // World +y: render +z, 90 deg.
        let (_, _, direction) = w.update(Some([0, 20 * 65536, 0]));
        assert!((direction - std::f64::consts::FRAC_PI_2).abs() < 1e-9);
    }
}
