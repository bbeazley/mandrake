use crate::clock::Instant;

use mandrake_net::endpoint::{Endpoint, HostOptions};
use mandrake_net::player_data::{JoinRequest, until_nul};
use mandrake_session::net::{NetEvent, NetRequest, Networking, Plugin};
use mandrake_session::{GameEvent, Request, Text};
use tracing::{info, warn};
use winit::event_loop::ActiveEventLoop;

use crate::app::myth2_scene::Myth2Scene;
use crate::app::shell::ShellHost;
use crate::app::shell_game::LoadFlow;
use crate::net_game::NetPlay;

fn installed_plugins(game_dir: &std::path::Path) -> Vec<Plugin> {
    let mut names: Vec<String> = crate::game_files::list_dir(&game_dir.join("plugins"))
        .iter()
        .filter_map(|p| mandrake_tag::tag_files::plugin_name(p))
        .collect();
    for name in mandrake_tag::tag_files::downloaded_plugins()
        .iter()
        .filter_map(|p| mandrake_tag::tag_files::plugin_name(p))
    {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    crate::app::recording::plugin_infos(game_dir, &names)
        .into_iter()
        .map(|p| Plugin {
            checksum: p.checksum,
            name: p.name.into_bytes(),
            url: p.url.into_bytes(),
        })
        .collect()
}

impl Myth2Scene {
    pub(super) fn wire_net_waker(&mut self, shell: &ShellHost) {
        let Some(waker) = shell.net_waker.clone() else {
            return;
        };
        if let Some(net) = self.pending_net.as_mut() {
            net.ensure_waker(&waker);
        }
        if let Some(net) = self.app.as_mut().and_then(|app| app.net.as_mut()) {
            net.ensure_waker(&waker);
        }
    }

    pub(super) fn net_wake(&mut self, shell: &ShellHost) {
        let map_drives = self.app.is_some() && self.load != LoadFlow::AttachGpu;
        if let Some(app) = self.app.as_mut().filter(|_| map_drives)
            && app.net.as_ref().is_some_and(NetPlay::is_loaded)
        {
            app.poll_network();
            return;
        }
        let in_map = self.app.as_ref().is_some_and(|app| app.net.is_some());
        if !in_map && let Some(net) = self.pending_net.as_mut() {
            net.window_occluded = Some(shell.window_facts.occluded);
            net.poll_lobby();
        }
    }

    fn net_play(&mut self) -> Option<&mut NetPlay> {
        match self.app.as_mut().and_then(|app| app.net.as_mut()) {
            Some(net) => Some(net),
            None => self.pending_net.as_mut(),
        }
    }

    fn owner_request(&mut self, request: &NetRequest) -> bool {
        use mandrake_net::lobby::code;
        use mandrake_net::ours::Ours;
        let Some(net) = self.net_play() else {
            return false;
        };
        let sent = match request {
            NetRequest::Description(d) | NetRequest::Options(d) => {
                net.as_owner(|j| j.send_ours(Ours::Description(d.to_bytes())))
            }
            NetRequest::Start { .. } | NetRequest::Restart => {
                net.as_owner(|j| j.owner_configure(code::START, 0))
            }
            NetRequest::Kick(id) => match u16::try_from(*id) {
                Ok(id) => net.as_owner(|j| j.send_ours(Ours::Boot { id })),
                Err(_) => false,
            },
            NetRequest::Lock(locked) => {
                net.as_owner(|j| j.send_ours(Ours::Lock { locked: *locked }))
            }
            NetRequest::Password(p) => net.as_owner(|j| j.send_ours(Ours::Password(p.0.clone()))),
            NetRequest::ForceEnd(kind) => {
                net.as_owner(|j| j.send_ours(Ours::ForceEnd { kind: *kind }))
            }
            _ => false,
        };
        if sent {
            let what = match request {
                NetRequest::Description(_) | NetRequest::Options(_) => {
                    "the description".to_string()
                }
                NetRequest::Start { .. } => "Start (code 100)".to_string(),
                NetRequest::Restart => "Restart Game (code 100)".to_string(),
                NetRequest::ForceEnd(kind) => format!("the forced end, kind {kind}"),
                other => format!("{other:?}"),
            };
            info!("net: owner: {what} to our server");
        }
        sent
    }

    pub(super) fn net_request(
        &mut self,
        shell: &mut ShellHost,
        request: Request,
        event_loop: &ActiveEventLoop,
    ) {
        if let Request::Net(r) = &request
            && self.owner_request(r)
        {
            return;
        }
        match request {
            Request::SaveNetPlayer(settings) => {
                let Some(path) = crate::settings::user_settings_path() else {
                    return;
                };
                if let Err(e) = crate::settings::save_net_settings(&path, &settings) {
                    warn!("settings: {}: the player wasn't saved: {e}", path.display());
                }
            }
            Request::Net(NetRequest::Join { address, request }) => {
                let text = String::from_utf8_lossy(&address.0).into_owned();
                #[cfg(not(target_arch = "wasm32"))]
                let connecting = anyhow::Ok(crate::net_game::Connecting::resolve_and_start(
                    text, *request, 0,
                ));
                #[cfg(target_arch = "wasm32")]
                let connecting = crate::web::Connecting::start(&text, *request)
                    .map_err(|e| anyhow::anyhow!("{text}: {e}"));
                match connecting {
                    Ok(c) => {
                        self.net_clock = Instant::now();
                        self.connecting = Some(c);
                    }
                    Err(e) => {
                        warn!("net: {e}");
                        self.net_events(shell, vec![NetEvent::TimedOut], event_loop);
                    }
                }
            }
            Request::Net(NetRequest::Ready(ready)) => {
                if let Some(net) = self.net_play() {
                    net.set_ready(ready);
                }
            }
            Request::Net(
                request @ (NetRequest::TeamButton(_)
                | NetRequest::TeamKick(_)
                | NetRequest::TeamLock),
            ) => {
                if let Some(net) = self.net_play()
                    && !net.team(&request)
                {
                    warn!("net: {request:?} sent nothing");
                }
            }
            Request::Net(NetRequest::Loaded) => {
                if let Some(net) = self.net_play() {
                    net.map_loaded();
                }
            }
            Request::Net(NetRequest::Standings) => {
                let report = self
                    .app
                    .as_ref()
                    .map(crate::app::ClientApp::standings)
                    .unwrap_or_default();
                info!(
                    "net: the loaded map's standings for endi: {} rows",
                    report.pages.as_ref().map_or(0, |p| p.rank.len())
                );
                self.net_events(
                    shell,
                    vec![NetEvent::Standings(Box::new(report))],
                    event_loop,
                );
            }
            Request::Net(NetRequest::MapOver) => {
                if let Some(net) = self.net_play() {
                    net.map_over();
                }
            }
            Request::Net(NetRequest::Leave) => {
                self.connecting = None;
                let net = match self.app.as_mut().and_then(|app| app.net.take()) {
                    Some(net) => Some(net),
                    None => self.pending_net.take(),
                };
                if let Some(net) = net {
                    info!("net: leaving the game");
                    net.leave();
                }
            }
            Request::Net(NetRequest::Host {
                port,
                password,
                player,
                description,
            }) => {
                self.connecting = None;
                if let Some(old) = self.pending_net.take() {
                    old.leave();
                }
                let options = HostOptions {
                    player: JoinRequest {
                        name: until_nul(&player.appearance.name).to_vec(),
                        password: Vec::new(),
                        player: *player,
                    },
                    password: password.0,
                    description: *description,
                };
                self.net_clock = Instant::now();
                match Endpoint::host(port, options, 0) {
                    Ok(endpoint) => {
                        info!("net: hosting on {:?}", endpoint.listen_addr());
                        let mut net = NetPlay::new(endpoint, self.net_clock, true);
                        net.set_internal_messages(self.internal_messages());
                        self.pending_net = Some(net);
                        self.net_events(shell, vec![NetEvent::Listening], event_loop);
                    }
                    Err(e) => {
                        warn!("net: listening on port {port}: {e}");
                        let reason = Text(e.to_string().into_bytes());
                        self.net_events(shell, vec![NetEvent::HostFailed(reason)], event_loop);
                    }
                }
            }
            Request::Net(NetRequest::Description(description)) => {
                if let Some(host) = self.net_play().and_then(NetPlay::host_mut) {
                    host.set_description(*description);
                }
            }
            Request::Net(NetRequest::Options(description)) => {
                if let Some(host) = self.net_play().and_then(NetPlay::host_mut) {
                    info!("net: host: the new game options; every Ready cleared");
                    host.set_options(*description);
                }
            }
            Request::Net(NetRequest::NetSave(path)) => {
                if let Some(net) = self.net_play()
                    && net.is_host()
                {
                    let loaded = match &path {
                        Some(path) => match crate::net_game::save::load_save(path) {
                            Ok(loaded) => {
                                info!(
                                    "net: the lobby plays the network save {}: its end tick {}",
                                    path.display(),
                                    loaded.0.ticks
                                );
                                Some(loaded)
                            }
                            Err(e) => {
                                warn!("net: the network save doesn't read: {e}");
                                None
                            }
                        },
                        None => None,
                    };
                    let (saved, restore) = loaded.unzip();
                    if path.is_none() {
                        info!("net: the lobby plays no network save");
                    }
                    net.host_save = restore;
                    if let Some(host) = net.host_mut() {
                        host.set_saved_game(saved);
                    }
                }
            }
            Request::Net(NetRequest::Start { vets }) => {
                if let Some(host) = self.net_play().and_then(NetPlay::host_mut) {
                    info!(
                        "net: host: Start (after a game: {}, vets {} bytes)",
                        vets.is_some(),
                        vets.as_ref().map_or(0, Vec::len)
                    );
                    host.set_after_game(vets.is_some());
                    host.set_vets(vets);
                    host.start(crate::net_game::machine_ticks());
                }
            }
            Request::Net(NetRequest::ForceEnd(kind)) => {
                if let Some(app) = self.app.as_mut() {
                    app.force_network_endgame(kind);
                } else if !self
                    .pending_net
                    .as_mut()
                    .is_some_and(|net| net.force_postgame_end(kind))
                {
                    warn!("net: the forced end, kind {kind}, with no map and no postgame");
                }
            }
            Request::Net(NetRequest::Restart) => {
                if let Some(host) = self.net_play().and_then(NetPlay::host_mut) {
                    info!("net: host: Restart Game");
                    host.restart_game();
                }
            }
            Request::Net(NetRequest::Kick(id)) => {
                if let Some(host) = self.net_play().and_then(NetPlay::host_mut)
                    && let Ok(id) = u8::try_from(id)
                {
                    info!("net: host: boot player {id}");
                    host.boot(id);
                }
            }
            Request::Net(NetRequest::Lock(locked)) => {
                if let Some(host) = self.net_play().and_then(NetPlay::host_mut) {
                    host.set_accepting(!locked);
                }
            }
            Request::Net(NetRequest::Chat(text)) => {
                if let Some(net) = self.net_play()
                    && !net.chat(mandrake_net::lobby::ChatTo::All, &text.0)
                {
                    warn!("net: the chat line wasn't sent");
                }
            }
            Request::NetChatTo(to, text) => {
                if let Some(net) = self.net_play()
                    && !net.chat(to, &text.0)
                {
                    warn!("net: the chat line to {to:?} wasn't sent");
                }
            }
            Request::Net(NetRequest::Password(password)) => {
                if let Some(host) = self.net_play().and_then(NetPlay::host_mut) {
                    host.set_password(password.0);
                }
            }
            Request::Net(NetRequest::TradeUnits(counts)) => {
                if let Some(app) = self.app.as_mut() {
                    info!("net: untr: the trade {counts:?}");
                    app.sim.queue_unit_adjustment(&counts);
                }
            }
            Request::Net(NetRequest::DownloadPlugins { url, plugins }) => {
                let settings = crate::settings::download_settings();
                let dir = mandrake_tag::tag_files::downloads_dir();
                match (self.pending_net.as_mut(), dir) {
                    (Some(net), Some(dir)) => {
                        let base = String::from_utf8_lossy(&url.0).into_owned();
                        info!(
                            "download: {} plugins from {base} ({{host}} {}) into {}",
                            plugins.len(),
                            net.joined_host,
                            dir.display()
                        );
                        net.download = Some(crate::net_game::download::Download::start(
                            &base,
                            &net.joined_host,
                            plugins,
                            dir,
                            settings.max_bytes,
                        ));
                    }
                    _ => {
                        warn!("download: no game or no downloads folder");
                        let done =
                            NetEvent::DownloadDone(Some(Text(b"no downloads folder".to_vec())));
                        self.net_events(shell, vec![done], event_loop);
                    }
                }
            }
            Request::Net(NetRequest::CancelDownload) => {
                if let Some(net) = self.pending_net.as_mut()
                    && let Some(d) = net.download.take()
                {
                    info!("download: cancelled");
                    d.cancel();
                }
            }
            Request::Net(NetRequest::ReloadPlugins) => {
                if let Some(menu) = self.menu.as_ref() {
                    info!("Reloading plugins from disk...");
                    let plugins = installed_plugins(menu.game_dir());
                    self.net_events(shell, vec![NetEvent::Plugins(plugins)], event_loop);
                }
            }
            Request::Net(NetRequest::SaveVeterans { name, table }) => {
                if let Some(menu) = self.menu.as_ref() {
                    let saved = mandrake_session::chat_commands::save_veterans(
                        menu.game_dir(),
                        &name.0,
                        &table,
                    );
                    info!("net: .savevets {}: {saved:?}", name.to_plain_utf8());
                    self.net_events(
                        shell,
                        vec![NetEvent::VeteransSaved { name, saved }],
                        event_loop,
                    );
                }
            }
            Request::Net(NetRequest::LoadVeterans { name }) => {
                if let Some(menu) = self.menu.as_ref() {
                    let table =
                        mandrake_session::chat_commands::load_veterans(menu.game_dir(), &name.0);
                    info!(
                        "net: .loadvets {}: {:?} veterans",
                        name.to_plain_utf8(),
                        table.as_ref().map(Vec::len)
                    );
                    self.net_events(
                        shell,
                        vec![NetEvent::VeteransLoaded { name, table }],
                        event_loop,
                    );
                }
            }
            Request::DescribeNetGame(mut game) => {
                if let Some(menu) = self.menu.as_ref() {
                    menu.describe_net_game(&mut game);
                }
                self.net_events(shell, vec![NetEvent::GameDescribed(game)], event_loop);
            }
            _ => {}
        }
    }

    fn internal_messages(&self) -> Vec<Vec<u8>> {
        self.menu
            .as_ref()
            .map(|m| crate::net_game::string_list(m.game_dir(), crate::net_game::INTERNAL_STRINGS))
            .unwrap_or_default()
    }

    fn net_events(
        &mut self,
        shell: &mut ShellHost,
        events: Vec<NetEvent>,
        event_loop: &ActiveEventLoop,
    ) {
        for event in events {
            let Some(menu) = self.menu.as_mut() else {
                return;
            };
            let requests = menu.game_event(GameEvent::Net(event));
            self.perform(shell, requests, event_loop);
        }
    }

    pub(super) fn net_step(&mut self, shell: &mut ShellHost, event_loop: &ActiveEventLoop) {
        if !self.net_announced
            && let Some(menu) = self.menu.as_ref()
        {
            self.net_announced = true;
            let game_dir = menu.game_dir().to_path_buf();
            let downloads = crate::settings::download_settings();
            if downloads.allow {
                mandrake_tag::tag_files::set_downloads_dir(crate::settings::downloads_dir());
            }
            #[allow(unused_mut)]
            let mut settings = crate::settings::user_settings_path()
                .and_then(|p| crate::settings::load_net_settings(&p).ok())
                .unwrap_or_else(crate::settings::net_defaults);
            #[cfg(target_arch = "wasm32")]
            if settings.address.0.is_empty() {
                settings.address = Text(crate::web::site_address().into_bytes());
            }
            let available = Networking {
                settings,
                plugins: installed_plugins(&game_dir),
                foundation_checksum: 0,
                allow_downloads: downloads.allow,
                #[cfg(target_arch = "wasm32")]
                server: Some(Text(crate::web::site_address().into_bytes())),
                #[cfg(not(target_arch = "wasm32"))]
                server: None,
            };
            self.net_events(
                shell,
                vec![NetEvent::Available(Box::new(available))],
                event_loop,
            );
        }
        if let Some(answer) = self.connecting.as_mut().and_then(|c| c.poll()) {
            #[cfg(not(target_arch = "wasm32"))]
            let host_name = self
                .connecting
                .take()
                .map(|c| c.host_name)
                .unwrap_or_default();
            #[cfg(target_arch = "wasm32")]
            let host_name = String::new();
            self.connecting = None;
            match answer {
                Ok(endpoint) => {
                    info!(
                        "net: connected in {} ms",
                        self.net_clock.elapsed().as_millis()
                    );
                    let mut net = NetPlay::new(endpoint, self.net_clock, true);
                    net.set_internal_messages(self.internal_messages());
                    net.joined_host = host_name;
                    self.pending_net = Some(net);
                }
                Err(e) => {
                    warn!("net: the connect: {e}");
                    self.net_events(shell, vec![NetEvent::TimedOut], event_loop);
                }
            }
        }
        self.wire_net_waker(shell);
        let occluded = shell.window_facts.occluded;
        let in_map = self.app.as_ref().is_some_and(|app| app.net.is_some());
        if !in_map && let Some(net) = self.pending_net.as_mut() {
            net.window_occluded = Some(occluded);
            net.poll_lobby();
        }
        let mut events = self
            .net_play()
            .map(NetPlay::take_session_events)
            .unwrap_or_default();
        events.extend(self.download_step());
        if !events.is_empty() {
            self.net_events(shell, events, event_loop);
        }
        self.trade_step(shell, event_loop);
    }

    fn download_step(&mut self) -> Vec<NetEvent> {
        let Some(polled) = self
            .pending_net
            .as_mut()
            .and_then(|n| n.download.as_mut())
            .map(|d| d.poll())
        else {
            return Vec::new();
        };
        let mut events: Vec<NetEvent> = polled
            .progress
            .map(NetEvent::DownloadProgress)
            .into_iter()
            .collect();
        if let Some(done) = polled.done {
            if let Some(net) = self.pending_net.as_mut() {
                net.download = None;
            }
            match &done {
                Ok(()) => info!("download: done; reloading plugins from disk..."),
                Err(e) => warn!("download: failed: {e}"),
            }
            if let Some(menu) = self.menu.as_ref() {
                events.push(NetEvent::Plugins(installed_plugins(menu.game_dir())));
            }
            events.push(NetEvent::DownloadDone(
                done.err().map(|e| Text(e.into_bytes())),
            ));
        }
        events
    }

    fn trade_step(&mut self, shell: &mut ShellHost, event_loop: &ActiveEventLoop) {
        let may_open = self
            .menu
            .as_ref()
            .is_some_and(|m| m.over_the_game() && !m.overlay_open());
        let rect = self.menu.as_ref().and_then(|m| m.modeless_rect());
        let Some(app) = self.app.as_mut() else {
            return;
        };
        let (dialog_y, ui_y) = (app.dialog_ui().axis_factors().1, app.ui.axis_factors().1);
        let drop = rect.map_or(0, |r| (f64::from(r[3]) * dialog_y / ui_y).ceil() as i32);
        app.set_view_drop(drop);
        let Some((offered, over)) = app.net.as_ref().map(|n| n.trading) else {
            return;
        };
        if offered && over {
            return;
        }
        if !offered
            && (app.sim.engine.game_options & mandrake_common::game_options::UNIT_TRADING == 0
                || !app.sim.trading_time())
        {
            if let Some(net) = app.net.as_mut() {
                net.trading = (true, true);
            }
            return;
        }
        let mut events = Vec::new();
        if !offered
            && may_open
            && let Some(rows) = app.sim.unit_trading()
        {
            let tags = app
                .map_end
                .as_ref()
                .and_then(crate::carnage_report::MapEnd::open_tags);
            let name = |unit: mandrake_common::FourCC| -> Vec<u8> {
                let Some(tags) = tags.as_ref() else {
                    return Vec::new();
                };
                let source = |g: [u8; 4], id| tags.get(mandrake_common::FourCC(g), id);
                let Some(mons) = tags
                    .get(mandrake_common::FourCC(*b"unit"), unit)
                    .filter(|u| u.len() >= 4)
                    .map(|u| u32::from_be_bytes([u[0], u[1], u[2], u[3]]))
                else {
                    return Vec::new();
                };
                mandrake_sim::statistics::monster_spelling(&source, mons, 2).unwrap_or_default()
            };
            let trading = mandrake_session::screen::UnitTrading {
                rows: rows
                    .iter()
                    .map(|r| mandrake_session::screen::TradeOffer {
                        name: Text(name(r.unit)),
                        adjustable: r.adjustable,
                        minimum: r.minimum,
                        maximum: r.maximum,
                        placed: r.placed,
                        cost: r.cost,
                    })
                    .collect(),
                counts: rows.iter().map(|r| r.placed).collect(),
                top: app.hud.as_ref().map_or(0, |h| h.status.rect.bottom),
            };
            info!(
                "net: untr opens at tick {} (local time {}): {:?}",
                app.sim.current_tick,
                app.game_speed.polls(),
                rows.iter()
                    .map(|r| (
                        r.unit.to_string(),
                        r.adjustable,
                        r.placed,
                        r.minimum,
                        r.maximum,
                        r.cost
                    ))
                    .collect::<Vec<_>>()
            );
            events.push(NetEvent::UnitTrading(Box::new(trading)));
        }
        let opened = offered || !events.is_empty();
        let ended = opened && !over && !app.sim.trading_time();
        if ended {
            info!(
                "net: untr: the planning time ended at tick {}",
                app.sim.current_tick
            );
            events.push(NetEvent::TradingOver);
        }
        if let Some(net) = app.net.as_mut() {
            net.trading = (opened, over || ended);
        }
        if !events.is_empty() {
            self.net_events(shell, events, event_loop);
        }
    }
}
