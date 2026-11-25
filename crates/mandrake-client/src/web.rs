use std::io;
use tracing_subscriber::EnvFilter;

pub(crate) struct ConsoleLine(pub(crate) Vec<u8>);

impl io::Write for ConsoleLine {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for ConsoleLine {
    fn drop(&mut self) {
        let line = String::from_utf8_lossy(&self.0);
        let line = line.trim_end();
        if !line.is_empty() {
            console(line);
        }
    }
}

pub fn set_audio_rate(rate: u32) {
    crate::audio_out::set_web_rate(rate);
}

pub fn mix_audio(out: &mut [f32]) -> bool {
    crate::audio_out::web_mix(out)
}

pub fn set_game_files(sink: fn(&str, Option<Vec<u8>>)) {
    if let Some(dir) = crate::tfl_live::our_films_dir() {
        crate::config_store::mirror_into_game_files(&dir, sink);
    }
}

pub fn console(line: &str) {
    web_sys::console::log_1(&web_sys::wasm_bindgen::JsValue::from_str(line));
}

pub fn start(words: Vec<String>) -> Result<(), String> {
    std::panic::set_hook(Box::new(|info| console(&format!("panic: {info}"))));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(
            "info,wgpu_core=warn,wgpu_hal=warn,naga=warn",
        ))
        .without_time()
        .with_ansi(false)
        .with_writer(|| ConsoleLine(Vec::new()))
        .try_init();
    let words = std::iter::once("mandrake".to_owned())
        .chain(words)
        .map(Into::into)
        .collect();
    let args = crate::conf::parse_args(words).map_err(|e| e.to_string())?;
    crate::run(args).map_err(|e| format!("{e:#}"))
}

mod net_ws {
    use std::io;

    use web_sys::wasm_bindgen::{JsCast, JsValue};

    fn call(name: &str, args: &[JsValue]) -> io::Result<JsValue> {
        let global = js_sys::global();
        let net = js_sys::Reflect::get(&global, &JsValue::from_str("m2Net"))
            .ok()
            .filter(|v| v.is_object())
            .ok_or_else(|| io::Error::other("the page has no m2Net"))?;
        let f: js_sys::Function = js_sys::Reflect::get(&net, &JsValue::from_str(name))
            .ok()
            .and_then(|f| f.dyn_into().ok())
            .ok_or_else(|| io::Error::other(format!("m2Net.{name}?")))?;
        let array = js_sys::Array::new();
        for a in args {
            array.push(a);
        }
        f.apply(&net, &array)
            .map_err(|e| io::Error::other(format!("m2Net.{name}: {e:?}")))
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum State {
        Connecting,
        Open,
        Closed,
    }

    #[derive(Debug)]
    pub(crate) struct Socket {
        id: f64,
    }

    impl Socket {
        pub(super) fn open(address: &str) -> io::Result<Socket> {
            let id = call("open", &[JsValue::from_str(address)])?
                .as_f64()
                .ok_or_else(|| io::Error::other("m2Net.open: no id"))?;
            Ok(Socket { id })
        }

        pub(super) fn url(&self) -> String {
            call("url", &[JsValue::from_f64(self.id)])
                .ok()
                .and_then(|v| v.as_string())
                .unwrap_or_default()
        }

        pub(super) fn state(&self) -> State {
            match call("state", &[JsValue::from_f64(self.id)])
                .ok()
                .and_then(|v| v.as_f64())
            {
                Some(0.0) => State::Connecting,
                Some(1.0) => State::Open,
                _ => State::Closed,
            }
        }

        pub(super) fn why(&self) -> String {
            call("why", &[JsValue::from_f64(self.id)])
                .ok()
                .and_then(|v| v.as_string())
                .unwrap_or_default()
        }
    }

    impl mandrake_net::transport::ByteStream for Socket {
        fn read(&mut self, out: &mut Vec<u8>) -> io::Result<()> {
            let got = call("take", &[JsValue::from_f64(self.id)])?;
            if let Some(bytes) = got.dyn_ref::<js_sys::Uint8Array>() {
                out.extend_from_slice(&bytes.to_vec());
            }
            match self.state() {
                State::Closed if got.is_null() || got.is_undefined() => {
                    Err(io::Error::new(io::ErrorKind::UnexpectedEof, self.why()))
                }
                _ => Ok(()),
            }
        }

        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            match self.state() {
                State::Open => {}
                State::Connecting => return Err(io::ErrorKind::WouldBlock.into()),
                State::Closed => return Err(io::Error::new(io::ErrorKind::BrokenPipe, self.why())),
            }
            let array = js_sys::Uint8Array::from(bytes);
            call("send", &[JsValue::from_f64(self.id), array.into()])?;
            Ok(bytes.len())
        }

        fn close(&mut self) {
            let _ = call("close", &[JsValue::from_f64(self.id)]);
        }
    }

    impl Drop for Socket {
        fn drop(&mut self) {
            let _ = call("close", &[JsValue::from_f64(self.id)]);
        }
    }
}

pub(crate) struct Connecting {
    socket: Option<net_ws::Socket>,
    request: Option<mandrake_net::player_data::JoinRequest>,
    since: crate::clock::Instant,
    kind: mandrake_net::game_kind::GameKind,
}

impl Connecting {
    pub(crate) fn start(
        address: &str,
        request: mandrake_net::player_data::JoinRequest,
    ) -> std::io::Result<Self> {
        Self::start_as(mandrake_net::game_kind::GameKind::Myth2, address, request)
    }

    pub(crate) fn start_as(
        kind: mandrake_net::game_kind::GameKind,
        address: &str,
        request: mandrake_net::player_data::JoinRequest,
    ) -> std::io::Result<Self> {
        let socket = net_ws::Socket::open(address)?;
        tracing::info!("net: connecting to {}", socket.url());
        Ok(Self {
            socket: Some(socket),
            request: Some(request),
            since: crate::clock::Instant::now(),
            kind,
        })
    }

    pub(crate) fn poll(&mut self) -> Option<std::io::Result<mandrake_net::endpoint::Endpoint>> {
        let state = self.socket.as_ref()?.state();
        match state {
            net_ws::State::Connecting
                if self.since.elapsed() < mandrake_net::transport::CONNECT_TIMEOUT =>
            {
                None
            }
            net_ws::State::Connecting => {
                self.socket = None;
                Some(Err(std::io::ErrorKind::TimedOut.into()))
            }
            net_ws::State::Closed => {
                let why = self.socket.take().map(|s| s.why()).unwrap_or_default();
                Some(Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionRefused,
                    why,
                )))
            }
            net_ws::State::Open => {
                let socket = self.socket.take()?;
                let request = self.request.take()?;
                Some(Ok(mandrake_net::endpoint::Endpoint::join_over_as(
                    self.kind,
                    Box::new(socket),
                    request,
                    0,
                )))
            }
        }
    }
}

pub(crate) fn site_address() -> String {
    use web_sys::wasm_bindgen::JsValue;
    js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("location"))
        .and_then(|l| js_sys::Reflect::get(&l, &JsValue::from_str("host")))
        .ok()
        .and_then(|h| h.as_string())
        .unwrap_or_default()
}

fn conf_path(conf: Option<&str>) -> Option<std::path::PathBuf> {
    match conf.filter(|c| !c.is_empty()) {
        Some(name) => Some(crate::conf::resolve(std::path::Path::new(name))),
        None => crate::conf::default_path(),
    }
}

pub fn net_player(conf: Option<&str>) -> (String, [u16; 8]) {
    let net = conf_path(conf)
        .and_then(|path| crate::settings::load_net_settings(&path).ok())
        .unwrap_or_else(crate::settings::net_defaults);
    let name = net.name.0.iter().map(|&b| char::from(b)).collect();
    (name, net.colors)
}

pub fn set_net_player(name: &str, colors: [u16; 8], conf: Option<&str>) -> Result<(), String> {
    let path = conf_path(conf).ok_or("no settings file")?;
    let mut net = crate::settings::load_net_settings(&path)?;
    net.name = mandrake_session::Text(crate::settings::typed_net_name(name));
    net.colors = colors;
    crate::settings::save_net_settings(&path, &net)
}

pub fn tfl_net_player(conf: Option<&str>) -> (String, [u16; 8]) {
    let net = conf_path(conf)
        .and_then(|path| crate::settings::load_tfl_net_settings(&path).ok())
        .unwrap_or_else(crate::settings::net_defaults);
    let name = net.name.0.iter().map(|&b| char::from(b)).collect();
    (name, net.colors)
}

pub fn set_tfl_net_player(name: &str, colors: [u16; 8], conf: Option<&str>) -> Result<(), String> {
    let path = conf_path(conf).ok_or("no settings file")?;
    let mut net = crate::settings::load_tfl_net_settings(&path)?;
    net.name = mandrake_session::Text(crate::settings::typed_net_name(name));
    net.colors = colors;
    crate::settings::save_tfl_net_settings(&path, &net)
}

pub fn install_conf(name: &str, shipped: &str) -> Result<(), String> {
    let path = conf_path(Some(name)).ok_or("no settings file")?;
    let text = match crate::config_store::read_to_string(&path) {
        Ok(old) => crate::conf::with_table_of(shipped, &old, "net")?,
        Err(_) => shipped.to_owned(),
    };
    crate::config_store::write(&path, &text).map_err(|e| e.to_string())
}

pub(crate) fn quit_to_page() {
    use web_sys::wasm_bindgen::{JsCast, JsValue};
    let quit = js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("m2Quit"))
        .ok()
        .and_then(|f| f.dyn_into::<js_sys::Function>().ok());
    if let Some(quit) = quit {
        let _ = quit.call0(&JsValue::NULL);
    }
}
