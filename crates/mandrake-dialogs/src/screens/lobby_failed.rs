use mandrake_session::Screen;
use mandrake_session::net::NetScreen;

use super::join_failed::{self, Notice};
use super::{OpenScreen, ScreenFrame, lobby};
use crate::core::DialogStack;
use crate::resources::Resources;

pub fn open(
    screen: &NetScreen<'_>,
    res: &mut Resources,
    stack: &mut DialogStack,
) -> Option<Box<dyn OpenScreen>> {
    let NetScreen::LobbyFailed(lobby, failed) = screen else {
        return None;
    };
    let under = lobby::open(&NetScreen::Lobby(*lobby), res, stack);
    let notice = Notice::Failed(failed.text.0.clone());
    join_failed::push(&notice, res, stack);
    Some(Box::new(Open { under, notice }))
}

struct Open {
    under: Option<Box<dyn OpenScreen>>,
    notice: Notice,
}

impl OpenScreen for Open {
    fn frame(&mut self, screen: &Screen<'_>, frame: &mut ScreenFrame<'_>) {
        let Screen::Net(NetScreen::LobbyFailed(lobby, _)) = screen else {
            return;
        };
        if let Some(under) = &mut self.under {
            under.frame(&Screen::Net(NetScreen::Lobby(*lobby)), frame);
        }
        join_failed::frame(&self.notice, &[], 0, frame);
    }
}
