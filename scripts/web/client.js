const V = new URL(import.meta.url).search;
const codeUrl = (path) => new URL(path + V, import.meta.url).href;
const {
  default: init,
  add_game_file, film_plugin_names, engine_tag_set, start_game, set_audio_rate, mix_audio,
  net_player_name, net_player_colors, set_net_player, install_conf, client_version,
  tfl_net_player_name, tfl_net_player_colors, set_tfl_net_player,
} = await import(codeUrl('./pkg/mandrake_web.js'));
const { M2WebAudio } = await import(codeUrl('./js/mandrake-web-audio.js'));
const { installNet } = await import(codeUrl('./js/mandrake-net-ws.js'));

const DIRS = ['tags', 'plugins', 'recordings', 'preferences'];
const HEAD = 128;
const params = new URLSearchParams(location.search);
const film = params.get('film') ?? 'salvation_vic_new';
const play = params.get('play');
let menu = params.has('menu');
const extra = (params.get('args') ?? '').split(' ').filter(Boolean);
const devGame = params.has('dev-game');
const autoplay = ['autoplay', 'film', 'play', 'menu'].some((k) => params.has(k));
const tfl = params.get('game') === 'tfl';
const SERVED = tfl ? { list: '/game/tfl/list', url: '/game/tfl/', opfs: 'served-tfl', vfs: '/tfl/' }
  : { list: '/game/list', url: '/game/', opfs: 'served', vfs: '/game/' };
const netPlayer = tfl
  ? { name: tfl_net_player_name, colors: tfl_net_player_colors, set: set_tfl_net_player }
  : { name: net_player_name, colors: net_player_colors, set: set_net_player };
async function servedDir(root, path, create) {
  const parts = path.split('/');
  const name = parts.pop();
  let dir = root;
  for (const p of parts) dir = await dir.getDirectoryHandle(p, { create });
  return { dir, name };
}

const $ = (id) => document.getElementById(id);
const statusEl = $('status');

const canvasConfigs = (window.m2CanvasConfigs = []);
const displayLine = () => {
  const q = (m) => matchMedia(m).matches;
  return `screen ${screen.width}x${screen.height}, colorDepth ${screen.colorDepth}, gamut `
    + `${q('(color-gamut: rec2020)') ? 'rec2020' : q('(color-gamut: p3)') ? 'p3' : 'srgb'}, `
    + `dynamic-range ${q('(dynamic-range: high)') ? 'high' : 'standard'}, `
    + `fullscreen ${document.fullscreenElement ? document.fullscreenElement.id || document.fullscreenElement.nodeName : 'no'}`;
};
if (globalThis.GPUCanvasContext) {
  const configure = GPUCanvasContext.prototype.configure;
  GPUCanvasContext.prototype.configure = function (config) {
    const c = this.canvas;
    const line = `canvas configure: ${c.id || 'offscreen'} ${c.width}x${c.height} ${config.format}, `
      + `colorSpace ${config.colorSpace ?? 'srgb'}, toneMapping ${config.toneMapping?.mode ?? 'standard'}, `
      + `alpha ${config.alphaMode ?? 'opaque'}, views [${[...(config.viewFormats ?? [])].join(',')}]; `
      + displayLine();
    canvasConfigs.push(line);
    console.log('[page] ' + line);
    return configure.call(this, config);
  };
}
document.addEventListener('fullscreenchange', () => console.log('[page] fullscreen change: ' + displayLine()));
const status = (s) => { statusEl.textContent += s + '\n'; console.log('[page] ' + s); };

const PRESETS = {
  original: 'mandrake.original.toml',
  'retro-modern': 'mandrake.modernretro.toml',
};
async function installPreset(preset) {
  const name = PRESETS[preset];
  const r = await fetch(codeUrl('./' + name), { cache: 'no-store' });
  if (!r.ok) throw new Error(`${name}: ${r.status} ${r.statusText}`);
  install_conf(name, await r.text());
  return ['--conf', name];
}

const w = (hex) => [0, 2, 4].map((i) => parseInt(hex.slice(i, i + 2), 16) * 0x101);
const COLOURS = [
  ['Plum', [0x5998, 0x08f3, 0x2f4b]],
  ['Gold', [0xc424, 0x8cb2, 0x1a27]],
  ['Crimson', w('b01818')],
  ['Scarlet', w('e83a20')],
  ['Orange', w('e8801c')],
  ['Yellow', w('ecd22c')],
  ['Olive', w('6c7424')],
  ['Forest', w('1e5a26')],
  ['Green', w('3caa3c')],
  ['Teal', w('1c8282')],
  ['Sky', w('5aa8e8')],
  ['Blue', w('2048c8')],
  ['Navy', w('14205c')],
  ['Violet', w('7a3cb4')],
  ['Rose', w('e070a8')],
  ['Brown', w('6c4424')],
  ['Bone', w('e4dcc4')],
  ['Grey', w('808080')],
  ['Charcoal', w('303030')],
  ['Black', [0, 0, 0]],
];
const DEFAULT_CHOICES = {
  preset: 'retro-modern', name: '', primary: COLOURS[0][1], secondary: COLOURS[1][1],
};

const STORE = 'myth2.landing';
function loadChoices() {
  try {
    const c = JSON.parse(localStorage.getItem(STORE) ?? 'null');
    if (c && typeof c === 'object') {
      const rgb = (v, d) => (Array.isArray(v) && v.length === 3
        && v.every((n) => Number.isInteger(n) && n >= 0 && n <= 0xffff) ? v : d);
      return {
        preset: PRESETS[c.preset] ? c.preset : DEFAULT_CHOICES.preset,
        name: typeof c.name === 'string' ? c.name : '',
        primary: rgb(c.primary, DEFAULT_CHOICES.primary),
        secondary: rgb(c.secondary, DEFAULT_CHOICES.secondary),
        saved: true,
      };
    }
  } catch {  }
  return { ...DEFAULT_CHOICES, saved: false };
}
function saveChoices(c) {
  try {
    localStorage.setItem(STORE, JSON.stringify({
      preset: c.preset, name: c.name, primary: c.primary, secondary: c.secondary,
    }));
  } catch {  }
}

if (devGame || params.has('post-log')) {
  for (const level of ['log', 'warn', 'error']) {
    const original = console[level].bind(console);
    console[level] = (...a) => {
      original(...a);
      fetch('/log', { method: 'POST', body: a.map(String).join(' ') }).catch(() => {});
    };
  }
  addEventListener('error', (e) => console.error('[page] error: ' + e.message));
  let rafs = 0;
  const raf = () => { rafs += 1; requestAnimationFrame(raf); };
  requestAnimationFrame(raf);
  setInterval(() => {
    console.log(`[page] alive, ${rafs} animation frames, sound ${audioMixed} frames mixed,`
      + ` peak ${audioPeak.toFixed(3)}`);
    rafs = 0; audioPeak = 0; audioMixed = 0;
  }, 5000);
  document.addEventListener('visibilitychange',
    () => console.log('[page] visibility: ' + document.visibilityState));
  for (const type of ['keydown', 'keyup', 'pointerdown', 'pointerup']) {
    addEventListener(type, (e) => console.log(`[page] ${type} ${e.key ?? e.button}`
      + ` at ${e.clientX ?? ''},${e.clientY ?? ''} on ${e.target.id || e.target.nodeName}`), true);
  }
  addEventListener('unhandledrejection', (e) => console.error('[page] rejection: ' + e.reason));
}

const mb = (n) => (n / 1e6).toFixed(1) + ' MB';
function progress(phase, done, total, detail) {
  $('progress').hidden = false;
  $('phase').textContent = phase;
  const pct = total > 0 ? Math.min(100, Math.floor((done / total) * 100)) : 100;
  $('percent').textContent = pct + '%';
  $('fill').style.width = pct + '%';
  $('bytes').textContent = detail ?? (total > 0 ? `${mb(done)} of ${mb(total)}` : '');
}

async function fetchBytes(url, size, onBytes) {
  const r = await fetch(url, { cache: 'no-store' });
  if (!r.ok) throw new Error(`${decodeURIComponent(url)}: HTTP ${r.status}`);
  if (!r.body) {
    const b = new Uint8Array(await r.arrayBuffer());
    onBytes(b.byteLength);
    return b;
  }
  let out = new Uint8Array(Math.max(size, 1));
  let n = 0;
  const reader = r.body.getReader();
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    if (n + value.byteLength > out.byteLength) {
      const grown = new Uint8Array(Math.max(out.byteLength * 2, n + value.byteLength));
      grown.set(out.subarray(0, n));
      out = grown;
    }
    out.set(value, n);
    n += value.byteLength;
    onBytes(value.byteLength);
  }
  if (n !== size) throw new Error(`${decodeURIComponent(url)}: ${n} bytes of ${size}`);
  return out.byteLength === n ? out : out.slice(0, n);
}

async function fromHandle(root) {
  const files = new Map();
  for (const d of DIRS) {
    let dir;
    try { dir = await root.getDirectoryHandle(d); } catch { continue; }
    for await (const [name, h] of dir.entries()) {
      if (h.kind !== 'file') continue;
      const f = await h.getFile();
      files.set(d + '/' + name, { size: f.size, file: async () => h.getFile() });
    }
  }
  return files;
}

function fromInput(list) {
  const files = new Map();
  for (const f of list) {
    const parts = f.webkitRelativePath.split('/').slice(1);
    if (parts.length === 2 && DIRS.includes(parts[0])) {
      files.set(parts.join('/'), { size: f.size, file: async () => f });
    }
  }
  return files;
}

async function fromDevServer() {
  const list = await (await fetch('/dev-game/list')).json();
  const files = new Map();
  for (const { path, size } of list) {
    const url = '/dev-game/' + path.split('/').map(encodeURIComponent).join('/');
    files.set(path, {
      size,
      file: async () => (await fetch(url)).blob(),
      bytes: (onBytes) => fetchBytes(url, size, onBytes),
      head: async (n) => (await fetch(url + '?head=' + n)).arrayBuffer(),
    });
  }
  return files;
}

async function fromServer(list) {
  const t0 = performance.now();
  progress('Checking the stored game data', 0, list.length, `${list.length} files`);
  const root = await navigator.storage.getDirectory();
  const served = await root.getDirectoryHandle(SERVED.opfs, { create: true });
  let versions = {};
  try {
    const f = await (await served.getFileHandle('versions.json')).getFile();
    versions = JSON.parse(await f.text());
  } catch { versions = {}; }
  const saveVersions = async () => {
    const out = await (await served.getFileHandle('versions.json', { create: true })).createWritable();
    await out.write(JSON.stringify(versions));
    await out.close();
  };
  if (navigator.storage.persist) navigator.storage.persist().catch(() => {});
  const files = new Map();
  const missing = [];
  let checked = 0;
  for (const entry of list) {
    const { path, size, version } = entry;
    const { dir, name } = await servedDir(served, path, true);
    let handle = null;
    if (versions[path] === version) {
      try {
        handle = await dir.getFileHandle(name);
        if ((await handle.getFile()).size !== size) handle = null;
      } catch { handle = null; }
    }
    if (handle) {
      const h = handle;
      files.set(path, { size, file: async () => h.getFile() });
    } else {
      missing.push({ ...entry, dir, name });
    }
    checked += 1;
    progress('Checking the stored game data', checked, list.length, `${checked} of ${list.length} files`);
  }
  const total = missing.reduce((n, e) => n + e.size, 0);
  let done = 0;
  for (const [i, { path, size, version, dir, name }] of missing.entries()) {
    const phase = `Downloading the game data (${i + 1} of ${missing.length})`;
    const show = () => progress(phase, done, total, `${mb(done)} of ${mb(total)} · ${path}`);
    show();
    const url = SERVED.url + path.split('/').map(encodeURIComponent).join('/');
    const bytes = await fetchBytes(url, size, (n) => { done += n; show(); });
    const handle = await dir.getFileHandle(name, { create: true });
    const out = await handle.createWritable();
    await out.write(bytes);
    await out.close();
    versions[path] = version;
    await saveVersions();
    files.set(path, { size, file: async () => handle.getFile() });
  }
  const listed = new Set(list.map((e) => e.path));
  for (const path of Object.keys(versions)) {
    if (listed.has(path)) continue;
    try {
      const { dir, name } = await servedDir(served, path, false);
      await dir.removeEntry(name);
    } catch {  }
    delete versions[path];
  }
  await saveVersions();
  status(`game data: ${missing.length} files fetched (${(total / 1e6).toFixed(0)} MB),`
    + ` ${list.length - missing.length} from the browser's storage, in`
    + ` ${((performance.now() - t0) / 1000).toFixed(1)} s`);
  return files;
}

async function servedList(url = SERVED.list) {
  try {
    const r = await fetch(url, { cache: 'no-store' });
    if (!r.ok || !(r.headers.get('content-type') ?? '').startsWith('application/json')) return null;
    const list = await r.json();
    return Array.isArray(list) && list.length ? list : null;
  } catch { return null; }
}

async function opfsGame(create) {
  const root = await navigator.storage.getDirectory();
  return root.getDirectoryHandle('game', { create });
}

async function fromOpfs() {
  try {
    const files = await fromHandle(await opfsGame(false));
    return files.size ? files : null;
  } catch { return null; }
}

async function needed(files) {
  const names = (dir) => [...files.keys()].filter((p) => p.startsWith(dir + '/'))
    .map((p) => p.slice(dir.length + 1)).sort();
  if (tfl) return [...files.keys()].sort();
  if (menu) {
    return [...files.keys()].filter((p) => !p.startsWith('recordings/')).sort();
  }
  const filmPath = 'recordings/' + film + '.m2rec';
  let filmPlugins = [];
  if (!play) {
    if (!files.has(filmPath)) throw new Error('no ' + filmPath + ' in the folder');
    const bytes = new Uint8Array(await (await files.get(filmPath).file()).arrayBuffer());
    filmPlugins = film_plugin_names(bytes);
  }
  const plugins = names('plugins');
  const heads = new Uint8Array(plugins.length * HEAD);
  for (const [i, n] of plugins.entries()) {
    const f = files.get('plugins/' + n);
    const head = f.head ? await f.head(HEAD) : await (await f.file()).slice(0, HEAD).arrayBuffer();
    heads.set(new Uint8Array(head), i * HEAD);
  }
  const pref = files.get('preferences/metaserver plugin');
  const metaserver = pref ? new Uint8Array(await (await pref.file()).arrayBuffer()) : undefined;
  const order = engine_tag_set(names('tags'), plugins, heads, filmPlugins, metaserver);
  const unity = plugins.filter((n) => /^Patch 1\.[45] Unity$/.test(n)).map((n) => 'plugins/' + n);
  const want = new Set([...order, ...unity, ...names('tags').map((n) => 'tags/' + n),
    ...names('preferences').map((n) => 'preferences/' + n)]);
  if (!play) want.add(filmPath);
  return [...want].filter((p) => files.has(p));
}

async function keep(files, paths) {
  const game = await opfsGame(true);
  const total = paths.reduce((n, p) => n + files.get(p).size, 0);
  let done = 0;
  for (const p of paths) {
    progress('Keeping your folder in this browser', done, total);
    const [d, name] = p.split('/');
    const dir = await game.getDirectoryHandle(d, { create: true });
    const out = await (await dir.getFileHandle(name, { create: true })).createWritable();
    await out.write(await files.get(p).file());
    await out.close();
    done += files.get(p).size;
  }
}

let engineStarted = false;

async function run(files, picked, words0) {
  const t0 = performance.now();
  const paths = await needed(files);
  const total = paths.reduce((n, p) => n + files.get(p).size, 0);
  status(`${paths.length} files, ${(total / 1e6).toFixed(0)} MB: ${paths.join(', ')}`);
  if (picked) {
    await keep(files, paths);
    status(`kept in the browser's storage in ${((performance.now() - t0) / 1000).toFixed(1)} s`);
  }
  const t1 = performance.now();
  let done = 0;
  for (const [i, p] of paths.entries()) {
    const f = files.get(p);
    const phase = `Loading the game data (${i + 1} of ${paths.length})`;
    progress(phase, done, total, `${mb(done)} of ${mb(total)} · ${p}`);
    const bytes = f.bytes
      ? await f.bytes((n) => { done += n; progress(phase, done, total, `${mb(done)} of ${mb(total)} · ${p}`); })
      : new Uint8Array(await (await f.file()).arrayBuffer());
    if (!f.bytes) done += f.size;
    add_game_file(SERVED.vfs + p, bytes);
  }
  status(`read in ${((performance.now() - t1) / 1000).toFixed(1)} s`);
  const words = tfl ? ['--game', 'tfl', '--tfl-data', '/tfl']
    : ['--archive', '/game/tags/small install'];
  if (tfl) {  } else if (play) words.push('--play', play);
  else if (!menu) words.push('--replay', '/game/recordings/' + film + '.m2rec');
  words.push(...words0, ...extra);
  status('start: ' + words.join(' '));
  progress('Starting the game', 1, 1, '');
  await startAudio();
  engineStarted = true;
  start_game(words);
  $('landing').hidden = !params.has('keep-status');
  const canvas = $('myth2');
  canvas.addEventListener('contextmenu', (e) => e.preventDefault());
  canvas.focus();
}

let audioPeak = 0;
let audioMixed = 0;
async function startAudio() {
  if (params.has('mute')) return;
  const game = {
    set_audio_sample_rate: (rate) => set_audio_rate(rate),
    mix_audio: (block) => {
      if (mix_audio(block)) {
        audioMixed += block.length / 2;
        for (const v of block) audioPeak = Math.max(audioPeak, Math.abs(v));
      }
    },
  };
  try {
    const audio = await M2WebAudio.attach(game, { workletUrl: codeUrl('./js/mandrake-audio-worklet.js') });
    status(`audio: ${audio.mode}, ${audio.context.sampleRate} Hz, ${audio.context.state}`);
  } catch (e) {
    status('audio: none (' + e.message + ')');
  }
}

globalThis.m2Quit = () => {
  const q = new URLSearchParams(location.search);
  for (const k of ['autoplay', 'film', 'play', 'menu', 'args']) q.delete(k);
  const s = q.toString().replace(/=(&|$)/g, '$1');
  setTimeout(() => location.replace(location.pathname + (s ? '?' + s : '')), 50);
};

const same = (a, b) => a.length >= 3 && b.length >= 3 && a[0] === b[0] && a[1] === b[1] && a[2] === b[2];
const css = ([r, g, b]) => `rgb(${r >> 8}, ${g >> 8}, ${b >> 8})`;

function swatches(id, chosen, onPick) {
  const box = $(id);
  const label = $(id + '-name');
  box.textContent = '';
  const list = COLOURS.some(([, c]) => same(c, chosen)) ? COLOURS
    : [['Your colour from the game', chosen], ...COLOURS];
  const buttons = list.map(([name, c]) => {
    const b = document.createElement('button');
    b.type = 'button';
    b.className = 'swatch';
    b.style.background = css(c);
    b.title = name;
    b.setAttribute('role', 'radio');
    b.setAttribute('aria-label', name);
    b.dataset.colour = c.join(',');
    b.onclick = () => select(b, name, c);
    box.append(b);
    return [b, name, c];
  });
  function select(b, name, c) {
    for (const [o] of buttons) o.setAttribute('aria-checked', String(o === b));
    label.textContent = name;
    onPick(c);
  }
  const [b, name, c] = buttons.find(([, , c]) => same(c, chosen));
  select(b, name, c);
}

function showGames(choices, tflServed) {
  for (const r of document.querySelectorAll('input[name="game"]')) {
    r.checked = r.value === (tfl ? 'tfl' : 'myth2');
    if (r.value === 'tfl' && !tflServed) {
      r.disabled = true;
      $('tfl-note').textContent = 'This site serves no Fallen Lords data.';
    }
    r.onchange = () => {
      readChoices(choices);
      saveChoices(choices);
      const q = new URLSearchParams(location.search);
      if (r.value === 'tfl') q.set('game', 'tfl'); else q.delete('game');
      const s = q.toString();
      location.replace(location.pathname + (s ? '?' + s : ''));
    };
  }
}

function showChoices(c) {
  for (const r of document.querySelectorAll('input[name="preset"]')) r.checked = r.value === c.preset;
  $('name').value = c.name;
  swatches('primary', c.primary, (v) => { c.primary = v; });
  swatches('secondary', c.secondary, (v) => { c.secondary = v; });
}

function readChoices(c) {
  c.preset = document.querySelector('input[name="preset"]:checked')?.value ?? DEFAULT_CHOICES.preset;
  c.name = $('name').value.replace(/[^\x20-\x7e]/g, '').trim().slice(0, 31);
  $('name').value = c.name;
  return c;
}

function showError(e) {
  const msg = e && e.message ? e.message : String(e);
  status('error: ' + msg);
  console.error(e);
  $('landing').hidden = false;
  $('error').hidden = false;
  $('error').textContent = (engineStarted ? 'The game could not start: ' : 'Could not load the game data: ')
    + msg;
  $('phase').textContent = 'Stopped';
  const play = $('play');
  play.disabled = false;
  play.textContent = 'Retry';
}

function missingNeeds() {
  if (!window.isSecureContext) {
    return {
      title: 'Open this page over https',
      text: `This page was opened over plain http (${location.origin}). Browsers give the game its `
        + 'storage and WebGPU only on a secure page, so it cannot run from here. Open the site\'s '
        + 'https address instead (or http://localhost on the machine that runs the server).',
    };
  }
  if (!navigator.gpu) {
    return {
      title: 'This browser has no WebGPU',
      text: 'The game draws with WebGPU, which this browser does not offer (or has turned off). '
        + 'Use a recent Chrome, Edge or Chromium on a computer; Firefox and Safari do not run '
        + 'the game yet.',
    };
  }
  if (!navigator.storage?.getDirectory) {
    return {
      title: 'This browser has no file storage',
      text: 'The game keeps its data in the browser\'s storage (OPFS), which this browser does '
        + 'not offer. Use a recent Chrome, Edge or Chromium.',
    };
  }
  return null;
}

function showNeeds(need) {
  status('cannot run: ' + need.title);
  $('needs-title').textContent = need.title;
  $('needs-text').textContent = need.text;
  $('needs').hidden = false;
  $('choices').hidden = true;
  $('links').hidden = true;
}

try {
  await init({ module_or_path: codeUrl('./pkg/mandrake_web_bg.wasm') });
  $('version').textContent = 'Version ' + client_version();
  const need = missingNeeds();
  if (need) {
    showNeeds(need);
    throw null;
  }
  installNet();
  if (tfl) {
    document.title = 'Myth: The Fallen Lords';
    document.querySelector('#landing h1').textContent = 'Myth: The Fallen Lords';
  }
  const list = devGame || params.has('own') ? null : await servedList();
  if (tfl && !list && !autoplay) {
    const q = new URLSearchParams(location.search);
    q.delete('game');
    const s = q.toString();
    location.replace(location.pathname + (s ? '?' + s : ''));
    throw null;
  }
  if (tfl && !list) throw new Error('this server has no TFL data (its tfl_dir)');
  const stored = devGame || list ? null : await fromOpfs();
  const source = devGame ? fromDevServer
    : list ? () => fromServer(list)
      : stored ? async () => stored
        : null;
  if (list) $('link-own').hidden = false;
  if (!devGame && !list) {
    $('link-own-folder').hidden = false;
    if (stored) $('forget').hidden = false;
  }
  $('forget').onclick = async () => {
    try { await (await navigator.storage.getDirectory()).removeEntry('game', { recursive: true }); } catch {  }
    location.reload();
  };

  if (autoplay) {
    if (list && !play && !params.has('film')) menu = true;
    if (devGame) status('dev game');
    if (!source) throw new Error('no game data: open the page without ?autoplay to choose a folder');
    await run(await source(), false, []);
  } else {
    menu = true;
    const choices = loadChoices();
    const gameName = netPlayer.name(PRESETS[choices.preset]);
    const gameColours = netPlayer.colors(PRESETS[choices.preset]);
    if (gameName) {
      choices.name = gameName.replace(/[^\x20-\x7e]/g, '');
      choices.primary = gameColours.slice(0, 3);
      choices.secondary = gameColours.slice(4, 7);
    } else if (!choices.saved) {
      choices.primary = gameColours.slice(0, 3);
      choices.secondary = gameColours.slice(4, 7);
    }
    showChoices(choices);
    showGames(choices, tfl ? !!list : !!(await servedList('/game/tfl/list')));
    const playButton = $('play');
    playButton.disabled = false;
    playButton.textContent = 'Play';
    const input = $('pick-input');
    let busy = false;
    const go = async (getFiles, picked) => {
      busy = true;
      playButton.disabled = true;
      playButton.textContent = 'Loading…';
      $('error').hidden = true;
      try {
        readChoices(choices);
        saveChoices(choices);
        const launch = new URLSearchParams({ preset: choices.preset, name: choices.name });
        fetch('/launch?' + launch, { method: 'POST', keepalive: true }).catch(() => {});
        const presetWords = await installPreset(choices.preset);
        netPlayer.set(choices.name, [...choices.primary, 0, ...choices.secondary, 0],
          PRESETS[choices.preset]);
        status(`preset ${choices.preset} (${PRESETS[choices.preset]}), name "${choices.name}"`);
        await run(await getFiles(), picked, presetWords);
      } catch (e) {
        showError(e);
      } finally {
        busy = false;
      }
    };
    input.onchange = () => go(async () => fromInput(input.files), true);
    $('choices').onsubmit = (e) => {
      e.preventDefault();
      if (busy) return;
      if (engineStarted) { location.reload(); return; }
      if (source) {
        go(source, false);
      } else if (window.showDirectoryPicker) {
        window.showDirectoryPicker({ mode: 'read' })
          .then((root) => go(() => fromHandle(root), true))
          .catch((err) => { if (err.name !== 'AbortError') showError(err); });
      } else {
        input.click();
      }
    };
  }
} catch (e) {
  if (e !== null) showError(e);
}
