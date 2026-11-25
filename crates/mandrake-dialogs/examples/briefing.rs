//! The briefing's canvas as PNGs, from the real catalog, by a fake clock (design section 9).
//!
//! ```text
//! cargo run -p mandrake-dialogs --example briefing -- OUT_PREFIX MESH TICK [TICK..]
//! cargo run -p mandrake-dialogs --example briefing -- --find FRAME.png MESH FROM_TICK TO_TICK
//! cargo run -p mandrake-dialogs --example briefing -- --end MESH
//! ```
//! A tick is from the briefing's first frame, and a PNG goes to `OUT_PREFIX-TICK.png`. `--find` prints the
//! tick whose picture differs least from a frame of the engine, as the engine's own timing is only
//! rough where it's captured. `--end` prints the tick the briefing ends on with nothing sounding, and what a
//! redraw costs: build with `--release` for numbers.

use std::path::Path;

use mandrake_common::FourCC;
use mandrake_dialogs::EngineDialogs;
use mandrake_session::{Action, Catalog, FrameInput, FrontEnd, Progress, Screen, Session};
use mandrake_tag::TagLookup;
use mandrake_tag::mesh_links::CampaignTags;
use mandrake_tag::tag_files::{INTERFACE_800X600, dialog_tag_files};

fn mesh_of(name: &str) -> FourCC {
    let mut id = *b"    ";
    id[..name.len()].copy_from_slice(name.as_bytes());
    FourCC(id)
}

struct Run {
    session: Session,
    front_end: EngineDialogs,
    tick: u64,
    sound_busy: bool,
}

impl Run {
    fn new(game: &Path, mesh: FourCC) -> Self {
        let files = dialog_tag_files(game, Some(INTERFACE_800X600));
        let catalog = Catalog::from_tags(&CampaignTags::open(game));
        let mut session = Session::new(catalog, Progress::default());
        session.act(Action::NewGame { all_levels: true });
        session.act(Action::ChooseLevel(mesh));
        session.act(Action::StartLevel);
        assert!(
            matches!(session.screen(), Screen::Briefing(_)),
            "{mesh} has no briefing: {:?}",
            session.screen()
        );
        Self {
            session,
            front_end: EngineDialogs::new(TagLookup::open(&files)),
            tick: 0,
            sound_busy: true,
        }
    }

    /// True when the briefing is over.
    fn frame(&mut self) -> bool {
        let input = FrameInput {
            now_ms: (self.tick * 1000).div_ceil(30),
            size: (800, 600),
            events: &[],
            sound_busy: self.sound_busy,
            shift_held: false,
        };
        let out = self.front_end.frame(&self.session.screen(), &input);
        out.actions.contains(&Action::BriefingDone)
    }

    fn run_to(&mut self, tick: u64) {
        while self.tick < tick {
            self.tick += 1;
            self.frame();
        }
    }
}

fn main() {
    let game = std::env::var("MYTH2_DATA_DIR").expect("MYTH2_DATA_DIR");
    let game = Path::new(&game);
    let mut args = std::env::args().skip(1);
    let first = args.next().expect("OUT_PREFIX or --find");
    if first == "--find" {
        let frame = image::open(args.next().expect("FRAME"))
            .expect("frame")
            .to_rgba8();
        let mesh = mesh_of(&args.next().expect("MESH"));
        let from: u64 = args.next().expect("FROM").parse().expect("FROM");
        let to: u64 = args.next().expect("TO").parse().expect("TO");
        let mut run = Run::new(game, mesh);
        run.frame();
        run.run_to(from);
        let mut best = (usize::MAX, 0);
        while run.tick <= to {
            let rgba = run.front_end.canvas().to_rgba();
            let differ = rgba
                .as_chunks::<4>()
                .0
                .iter()
                .zip(frame.as_raw().as_chunks::<4>().0)
                .filter(|(a, b)| a[..3] != b[..3])
                .count();
            if differ < best.0 {
                best = (differ, run.tick);
            }
            run.tick += 1;
            run.frame();
        }
        println!("tick {}: {} differ", best.1, best.0);
        return;
    }
    if first == "--end" {
        let mesh = mesh_of(&args.next().expect("MESH"));
        let mut run = Run::new(game, mesh);
        run.sound_busy = false;
        let (mut redraws, mut worst) = (0u32, std::time::Duration::ZERO);
        let start = std::time::Instant::now();
        loop {
            let revision = run.front_end.revision();
            let began = std::time::Instant::now();
            let done = run.frame();
            if run.front_end.revision() != revision {
                redraws += 1;
                worst = worst.max(began.elapsed());
            }
            if done || run.tick > 100_000 {
                println!(
                    "ends on tick {}; {redraws} redraws, {:?} each, the worst {worst:?}",
                    run.tick,
                    start.elapsed() / redraws.max(1),
                );
                return;
            }
            run.tick += 1;
        }
    }
    let mesh = mesh_of(&args.next().expect("MESH"));
    let mut run = Run::new(game, mesh);
    run.frame();
    for tick in args {
        let tick: u64 = tick.parse().expect("TICK");
        run.run_to(tick);
        let canvas = run.front_end.canvas();
        image::save_buffer(
            format!("{first}-{tick}.png"),
            &canvas.to_rgba(),
            canvas.width as u32,
            canvas.height as u32,
            image::ColorType::Rgba8,
        )
        .expect("write");
    }
}
