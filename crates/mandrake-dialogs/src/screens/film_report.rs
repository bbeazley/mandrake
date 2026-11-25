use mandrake_common::FourCC;
use mandrake_session::{Action, FilmPages, PostgameReport, Screen};

use super::{OpenScreen, ScreenFrame};
use crate::controls::button::button_flags;
use crate::controls::carnage::{CarnageText, Page};
use crate::controls::list::list_flags;
use crate::core::{Dialog, DialogStack, dialog_flags};
use crate::resources::{BAIE, Resources};

pub const POCR: FourCC = FourCC(*b"pocr");
const BACKGROUND: FourCC = FourCC(*b"infs");
pub const OK: i16 = 1;
const STATS: i16 = 2;
const GRAPH: i16 = 3;
const HEADER: i16 = 4;
pub const FIRST_TAB: i16 = 5;
pub const LAST_TAB: i16 = 11;
const TAB_SEQUENCE: i32 = 10;

#[derive(Debug, Clone, Copy)]
pub(super) struct CarnageItems {
    pub stats: i16,
    pub graph: i16,
    pub header: i16,
    pub first_tab: i16,
    pub frame: bool,
}

const POCR_ITEMS: CarnageItems = CarnageItems {
    stats: STATS,
    graph: GRAPH,
    header: HEADER,
    first_tab: FIRST_TAB,
    frame: true,
};

impl CarnageItems {
    const fn last_tab(self) -> i16 {
        self.first_tab + 6
    }

    const fn page_of(self, tab: i16) -> Page {
        match tab - self.first_tab {
            0 => Page::Stats(0),
            1 => Page::Stats(1),
            t => Page::Graph(t - 2),
        }
    }
}

fn valid(pages: &FilmPages, page: Page) -> bool {
    match page {
        Page::Stats(0) => true,
        Page::Stats(_) => pages.perm.is_some(),
        Page::Graph(g) => usize::try_from(g)
            .ok()
            .and_then(|g| pages.graphs.get(g))
            .is_some_and(Option::is_some),
    }
}

pub fn open(
    data: &PostgameReport,
    res: &mut Resources,
    stack: &mut DialogStack,
) -> Option<Box<dyn OpenScreen>> {
    let Some(def) = res.dialog(POCR) else {
        return Some(Box::new(NoDialog { answered: false }));
    };
    let mut dialog = Dialog::new(&def, POCR, dialog_flags::MODAL, res);
    dialog.set_default_item(OK);
    if let Some(art) = res.art(BACKGROUND) {
        dialog.set_background(Some(art), 0);
    }
    let open = Report(Carnage::open(
        &mut dialog,
        POCR_ITEMS,
        data.pages.clone().unwrap_or_default(),
        res,
    ));
    tracing::info!("film report: the carnage report opens");
    stack.push(dialog);
    Some(Box::new(open))
}

pub(super) struct Carnage {
    items: CarnageItems,
    text: CarnageText,
    pages: FilmPages,
}

impl Carnage {
    pub(super) fn open(
        dialog: &mut Dialog,
        items: CarnageItems,
        pages: FilmPages,
        res: &mut Resources,
    ) -> Self {
        if items.frame
            && let Some(list) = dialog.control_mut(items.stats).and_then(|c| c.list_mut())
        {
            list.set_flag(list_flags::FRAME, true);
        }
        let baie = res.art(BAIE);
        for tab in items.first_tab..=items.last_tab() {
            if let Some((item, button)) = dialog.control_mut(tab).and_then(|c| c.button_mut()) {
                button.set_flags(button_flags::STICKS);
                button.set_shape(item, baie.clone(), TAB_SEQUENCE);
            }
        }
        for tab in items.first_tab..=items.last_tab() {
            dialog.set_enabled(tab, valid(&pages, items.page_of(tab)));
        }
        let mut carnage = Self {
            items,
            text: CarnageText::new(res),
            pages,
        };
        carnage.show(dialog, items.first_tab);
        carnage
    }

    pub(super) fn is_tab(&self, item: i16) -> bool {
        (self.items.first_tab..=self.items.last_tab()).contains(&item)
    }

    pub(super) fn show(&mut self, dialog: &mut Dialog, tab: i16) {
        let CarnageItems {
            stats,
            graph,
            header,
            ..
        } = self.items;
        let page = self.items.page_of(tab);
        if valid(&self.pages, page) {
            match page {
                Page::Stats(n) => {
                    dialog.set_visible(stats, true);
                    dialog.set_visible(graph, false);
                    let rows = dialog
                        .control(stats)
                        .map(|c| self.text.stats_rows(&self.pages, n, c.item.rect));
                    if let (Some(rows), Some(control)) = (rows.clone(), dialog.control_mut(stats)) {
                        control.item.dirty = true;
                        if let Some(list) = control.list_mut() {
                            list.set_carnage_rows(rows);
                        }
                    }
                    if let Some((item, h)) = dialog
                        .control_mut(header)
                        .and_then(|c| c.carnage_header_mut())
                    {
                        h.set_page(item, page, rows.as_ref());
                    }
                }
                Page::Graph(g) => {
                    dialog.set_visible(stats, false);
                    dialog.set_visible(graph, true);
                    let shown = self.pages.graphs.get(g as usize).cloned().flatten();
                    if let Some((item, control)) = dialog
                        .control_mut(graph)
                        .and_then(|c| c.carnage_graph_mut())
                    {
                        control.set_graph(item, g, shown);
                    }
                    if let Some((item, h)) = dialog
                        .control_mut(header)
                        .and_then(|c| c.carnage_header_mut())
                    {
                        h.set_page(item, page, None);
                    }
                }
            }
        }
        for t in self.items.first_tab..=self.items.last_tab() {
            if let Some((item, button)) = dialog.control_mut(t).and_then(|c| c.button_mut()) {
                button.set_state(item, i32::from(t == tab));
            }
        }
    }
}

struct Report(Carnage);

impl OpenScreen for Report {
    fn frame(&mut self, screen: &Screen<'_>, frame: &mut ScreenFrame<'_>) {
        let Screen::FilmReport(_) = screen else {
            return;
        };
        let Some(dialog) = frame.stack.find_mut(POCR) else {
            return;
        };
        for hit in frame.hits.iter().filter(|h| h.dialog == POCR) {
            match hit.item {
                OK => frame.out.actions.push(Action::CloseReport),
                tab if self.0.is_tab(tab) => self.0.show(dialog, tab),
                _ => {}
            }
        }
    }
}

struct NoDialog {
    answered: bool,
}

impl OpenScreen for NoDialog {
    fn frame(&mut self, _screen: &Screen<'_>, frame: &mut ScreenFrame<'_>) {
        if !std::mem::replace(&mut self.answered, true) {
            tracing::warn!("film report: no pocr dialog");
            frame.out.actions.push(Action::CloseReport);
        }
    }
}
