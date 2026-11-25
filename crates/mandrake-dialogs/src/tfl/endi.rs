use mandrake_common::FourCC;
use mandrake_session::Screen;

use crate::core::DialogStack;
use crate::resources::Resources;
use crate::screens::OpenScreen;
use crate::screens::net_postgame::{self, ENDI};

pub const MEGR: FourCC = FourCC(*b"megr");

pub fn open(
    screen: &Screen<'_>,
    res: &mut Resources,
    stack: &mut DialogStack,
) -> Option<Box<dyn OpenScreen>> {
    let Screen::Net(net) = screen else {
        return None;
    };
    let style = net_postgame::Style {
        rows: crate::tfl::lobby::player_rows,
        back_log: false,
    };
    let open = net_postgame::open_with(net, res, stack, style)?;
    if let Some(d) = stack.find_mut(ENDI) {
        d.set_background(res.art(MEGR), 0);
    }
    Some(open)
}
