use super::*;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("m2-conf-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn words(list: &[&str]) -> Vec<OsString> {
    std::iter::once("mandrake")
        .chain(list.iter().copied())
        .map(OsString::from)
        .collect()
}

#[test]
fn a_missing_conf_file_is_an_error() {
    let dir = scratch("missing");
    let path = dir.join("nothing.toml");
    let e = parse_args(words(&["--conf", path.to_str().unwrap()])).unwrap_err();
    let shown = e.to_string();
    assert!(
        shown.contains("no such file") && shown.contains("nothing.toml"),
        "{shown}"
    );
    assert!(parse_args(words(&["--headless"])).is_ok());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_flags_beat_the_file_and_the_file_beats_the_defaults() {
    use crate::args::{Fog, Lighting, Water};
    use crate::ui_scale::{Sampling, UiFilter};
    let dir = scratch("order");
    let path = dir.join("mandrake.modernretro.toml");
    std::fs::write(
        &path,
        "[display]\nui_filter = \"lanczos2\"\nsprite_filter = \"sharp\"\nsampling = \"software\"\n\
         lighting = true\nfog = \"on\"\nui_scale = 200\nwater = \"enhanced\"\n\
         water_maps = [\"85tl\", \"85tk\"]\nbad = 1\n",
    )
    .unwrap();
    let conf = path.to_str().unwrap();
    let none = parse_args(words(&["--headless"])).unwrap();
    assert_eq!(
        (none.sampling, none.lighting, none.fog, none.ui_filter),
        (Sampling::Software, None, None, None)
    );
    let file = parse_args(words(&["--conf", conf, "--headless"])).unwrap();
    assert_eq!(file.conf.as_deref(), Some(path.as_path()));
    assert_eq!(file.ui_filter, Some(UiFilter::Lanczos2));
    assert_eq!(file.sprite_filter, Some(UiFilter::Sharp));
    assert_eq!(file.lighting, Some(Lighting::On));
    assert_eq!(file.fog, Some(Fog::On));
    assert_eq!(file.water, Some(Water::Enhanced));
    assert_eq!(file.water_maps, ["85tl", "85tk"]);
    assert!(file.ui_scale.is_some());
    let flags = parse_args(words(&[
        "--sampling",
        "linear",
        "--conf",
        conf,
        "--lighting",
        "off",
        "--fog",
        "engine",
    ]))
    .unwrap();
    assert_eq!(flags.sampling, Sampling::Linear);
    assert_eq!(flags.lighting, Some(Lighting::Off));
    assert_eq!(flags.fog, Some(Fog::Engine));
    assert_eq!(
        flags.ui_filter,
        Some(UiFilter::Lanczos2),
        "the file's still"
    );
    std::fs::write(&path, "[display]\nsampling = \"blurry\"\nfog = \"off\"\n").unwrap();
    let bad = parse_args(words(&["--conf", conf])).unwrap();
    assert_eq!(
        (bad.sampling, bad.fog),
        (Sampling::Software, Some(Fog::Off))
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_shipped_presets_parse() {
    use crate::ui_scale::{Sampling, UiFilter};
    let dir = scratch("presets");
    for (name, text) in [
        (
            "mandrake.original.toml",
            include_str!("../../mandrake.original.toml"),
        ),
        (
            "mandrake.modernretro.toml",
            include_str!("../../mandrake.modernretro.toml"),
        ),
    ] {
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        let args = parse_args(words(&["--conf", path.to_str().unwrap()])).unwrap();
        assert_eq!(args.water, Some(crate::args::Water::Enhanced), "{name}");
        assert!(crate::args::water_enhanced_on(&args, "85tl"), "{name}");
        assert!(!crate::args::water_enhanced_on(&args, "01bt"), "{name}");
        let interface = crate::interface_config::load(args.conf.as_deref(), true);
        if name.contains("original") {
            assert_eq!(args.sampling, Sampling::Linear);
            assert_eq!(args.ui_filter, Some(UiFilter::Nearest));
            assert!(interface.control_bar);
        } else {
            assert_eq!(args.ui_filter, Some(UiFilter::Lanczos2));
            assert_eq!(args.lighting, Some(crate::args::Lighting::On));
            assert!(!interface.control_bar);
        }
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn writing_back_keeps_an_unrelated_comment() {
    let mine = "# my setup\n[display]\nui_zoom = 125 # Ctrl +/-\nvsync = true\n\n\
                [controls.bindings]\n# camera\nmove_forward = \"W\"   # forward\n\n\
                [net]\nname = \"Old\"\n\n# keep me\n[fog.default]\nopacity = 0.2\n";
    let zoom = with_values(mine, &["display"], &[("ui_zoom", 200i64.into())]).unwrap();
    assert_eq!(zoom, mine.replace("ui_zoom = 125", "ui_zoom = 200"));
    let bound = with_values(
        &zoom,
        &["controls", "bindings"],
        &[("move_forward", "Up Arrow".into())],
    )
    .unwrap();
    assert_eq!(
        bound,
        zoom.replace("move_forward = \"W\"", "move_forward = \"Up Arrow\"")
    );
    let named = with_values(&bound, &["net"], &[("name", "Ranger".into())]).unwrap();
    assert_eq!(named, bound.replace("name = \"Old\"", "name = \"Ranger\""));
    assert!(named.contains("# keep me\n[fog.default]"));
    let made = with_values(
        mine,
        &["tfl", "controls", "bindings"],
        &[("quit", "Q".into())],
    )
    .unwrap();
    assert!(made.starts_with(mine), "{made}");
    assert!(
        made.ends_with("\n[tfl.controls.bindings]\nquit = \"Q\"\n"),
        "{made}"
    );
    let vsync = with_values(mine, &["display"], &[("fps_cap", 144i64.into())]).unwrap();
    assert_eq!(
        vsync,
        mine.replace("[display]\n", "[display]\nfps_cap = 144\n")
    );
    assert!(with_values("x = = 1", &["display"], &[("ui_zoom", 100i64.into())]).is_err());

    let dir = scratch("write");
    let path = dir.join("mandrake.toml");
    set_values(&path, &["display"], &[("ui_zoom", 150i64.into())]).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.starts_with('#') && text.ends_with("[display]\nui_zoom = 150\n"),
        "{text}"
    );
    std::fs::write(&path, mine).unwrap();
    set_values(&path, &["display"], &[("ui_zoom", 125i64.into())]).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), mine);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_written_defaults_are_the_shipped_file() {
    let dir = scratch("defaults");
    let path = dir.join("sub").join("mandrake.toml");
    write_defaults(&path).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), DEFAULT_CONF_TOML);
    let args = parse_args(words(&["--conf", path.to_str().unwrap(), "--headless"])).unwrap();
    assert_eq!(args.sampling, crate::ui_scale::Sampling::Software);
    assert_eq!(args.fog, None);
    assert_eq!(
        crate::interface_config::load(args.conf.as_deref(), true),
        crate::interface_config::InterfaceConfig::defaults()
    );
    let (fog, _, d) = crate::fog_config::load_native(args.conf.as_deref(), true);
    assert_eq!(d, []);
    let shipped = crate::fog_config::FogConfig::defaults();
    for mesh in ["cr01", "85tl", "00tm"] {
        assert_eq!(
            fog.resolve(mesh, &Default::default()),
            shipped.resolve(mesh, &Default::default())
        );
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_reinstalled_preset_keeps_the_net_table() {
    let shipped = "# preset\n[display]\nsampling = \"linear\"\n";
    let old = "[display]\nsampling = \"sharp\"\n[net]\nname = \"Ranger\"\n";
    let new = with_table_of(shipped, old, "net").unwrap();
    assert!(new.starts_with(shipped), "{new}");
    let net = crate::settings::parse_net_settings(&new, crate::settings::net_defaults()).unwrap();
    assert_eq!(net.name.0, b"Ranger");
    assert_eq!(with_table_of(shipped, "", "net").unwrap(), shipped);
}

#[test]
fn the_old_look_keys_are_converted() {
    use crate::args::{Fog, Lighting, TflExtras};
    let old = "# my setup\n[display]\nfog = \"on\" # Myth II's\n# TFL\n\
               tfl_fog = false\ntfl_lighting = true # lit\ntfl_detail_shadows = false\n\
               tfl_smoothing = false # tick by tick\ntfl_hover_frames = true\n\n\
               [interface]\ncontrol_bar = false # mine\n";
    let (new, moved) = with_new_look_keys(old).unwrap();
    assert_eq!(
        new,
        "# my setup\n[display]\nfog = \"on\" # Myth II's\n# TFL\n\
         lighting = true # lit\ndetail_shadows = false\n\n\
         [display.tfl]\nsmoothing = false # tick by tick\nhover_frames = true\n\n\
         [interface]\ncontrol_bar = false # mine\n"
    );
    assert_eq!(
        moved,
        vec![
            ("tfl_fog", None),
            ("tfl_lighting", Some("[display] lighting".to_owned())),
            (
                "tfl_detail_shadows",
                Some("[display] detail_shadows".to_owned())
            ),
            ("tfl_smoothing", Some("[display.tfl] smoothing".to_owned())),
            (
                "tfl_hover_frames",
                Some("[display.tfl] hover_frames".to_owned())
            ),
        ]
    );
    assert!(with_new_look_keys(&new).is_none(), "once");
    let (new, _) = with_new_look_keys(
        "[display.tfl]\nhover_frames = false\n[display]\ntfl_smoothing = true\ntfl_hover_frames = true",
    )
    .unwrap();
    assert_eq!(
        new,
        "[display.tfl]\nsmoothing = true\nhover_frames = false\n[display]\n"
    );
    let (new, _) = with_new_look_keys("[display]\ntfl_smoothing = false\n").unwrap();
    assert_eq!(new, "[display]\n\n[display.tfl]\nsmoothing = false\n");
    let (new, linear) = with_new_look_keys(
        "[display]\nsampling = \"linear\"\ntfl_fog = false # none\ntfl_lighting = false\n",
    )
    .unwrap();
    assert_eq!(
        new,
        "[display]\nsampling = \"linear\"\nfog = \"engine\" # none\nlighting = false\n"
    );
    assert_eq!(linear[0].1.as_deref(), Some("[display] fog = \"engine\""));
    assert!(with_new_look_keys("[display]\nfog = true\n").is_none());
    assert!(with_new_look_keys("[net]\ntfl_fog = true\n").is_none());
    let notice = old_keys_notice(Path::new("/c/m.toml"), &moved, true);
    assert!(notice.starts_with("settings: /c/m.toml: old keys: tfl_fog is dropped"));
    assert!(notice.contains("tfl_smoothing is now [display.tfl] smoothing"));
    assert!(notice.ends_with("rewritten in place"));

    let dir = scratch("old-look");
    let path = dir.join("mandrake.toml");
    let conf = path.to_str().unwrap();
    std::fs::write(&path, old).unwrap();
    let args = parse_args(words(&["--conf", conf, "--headless", "--game", "tfl"])).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), old);
    assert_eq!(args.fog, Some(Fog::On));
    assert_eq!(args.lighting, Some(Lighting::On));
    assert!(!TflExtras::of(&args).detail_shadows);
    let _ = parse_args(words(&["--conf", conf, "--game", "tfl"])).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("# my setup\n") && text.contains("control_bar = false # mine"));
    assert!(!text.contains("tfl_"), "{text}");
    assert_eq!(crate::settings::load_tfl_smoothing(&path), Ok(Some(false)));
    assert_eq!(
        crate::settings::load_tfl_hover_frames(&path),
        Ok(Some(true))
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_window_keys_are_preset_keys() {
    use crate::app::fps_cap::FpsCap;
    use crate::args::{DetailTextures, TenBit, Vsync};
    let dir = scratch("window-keys");
    let path = dir.join("mandrake.toml");
    let conf = path.to_str().unwrap();
    let read = |text: &str, extra: &[&str]| {
        std::fs::write(&path, text).unwrap();
        let mut all = vec!["--conf", conf];
        all.extend_from_slice(extra);
        parse_args(words(&all)).unwrap()
    };
    let none = read("[display]\nui_zoom = 150\n", &[]);
    assert_eq!(
        (none.detail_textures, none.vsync, none.ten_bit, none.fps_cap),
        (None, None, None, None)
    );
    assert_eq!(crate::startup::vsync_wanted(&none), (false, "the default"));
    assert_eq!(crate::startup::ten_bit_wanted(&none), (true, "the default"));
    assert_eq!(
        crate::startup::fps_cap_wanted(&none),
        (FpsCap::Auto, "the default")
    );
    let file = read(
        "[display]\ndetail_textures = true\nvsync = \"on\"\nten_bit = false\nfps_cap = 144\n",
        &[],
    );
    assert_eq!(file.detail_textures, Some(DetailTextures::On));
    assert!(crate::startup::detail_textures_wanted(&file));
    assert_eq!(
        crate::startup::vsync_wanted(&file),
        (true, "the settings file")
    );
    assert_eq!(
        crate::startup::ten_bit_wanted(&file),
        (false, "the settings file")
    );
    assert_eq!(
        crate::startup::fps_cap_wanted(&file),
        (FpsCap::Fixed(144), "the settings file")
    );
    let headless = read("[display]\ndetail_textures = \"on\"\n", &["--headless"]);
    assert_eq!(headless.detail_textures, Some(DetailTextures::On));
    assert_eq!(
        parse_args(words(&["--headless"])).unwrap().detail_textures,
        None
    );
    let flags = read(
        "[display]\nvsync = true\nten_bit = \"off\"\nfps_cap = \"off\"\n",
        &["--vsync", "off", "--ten-bit", "on", "--fps-cap", "auto"],
    );
    assert_eq!(crate::startup::vsync_wanted(&flags), (false, "--vsync"));
    assert_eq!(crate::startup::ten_bit_wanted(&flags), (true, "--ten-bit"));
    assert_eq!(
        crate::startup::fps_cap_wanted(&flags),
        (FpsCap::Auto, "--fps-cap")
    );
    for (text, cap) in [
        ("\"auto\"", FpsCap::Auto),
        ("\"off\"", FpsCap::Off),
        ("0", FpsCap::Off),
        ("\"144\"", FpsCap::Fixed(144)),
    ] {
        let a = read(&format!("[display]\nfps_cap = {text}\n"), &[]);
        assert_eq!(a.fps_cap, Some(cap), "{text}");
    }
    let off = read(
        "[display]\ndetail_textures = \"off\"\nvsync = false\nten_bit = \"on\"\n",
        &[],
    );
    assert_eq!(
        (off.detail_textures, off.vsync, off.ten_bit),
        (
            Some(DetailTextures::Off),
            Some(Vsync::Off),
            Some(TenBit::On)
        )
    );
    for bad in [
        "detail_textures = 1",
        "detail_textures = \"yes\"",
        "vsync = 1",
        "ten_bit = 10",
        "fps_cap = -5",
        "fps_cap = true",
        "fps_cap = \"fast\"",
    ] {
        let a = read(&format!("[display]\n{bad}\nfog = \"off\"\n"), &[]);
        assert_eq!(
            (a.detail_textures, a.vsync, a.ten_bit, a.fps_cap),
            (None, None, None, None),
            "{bad}"
        );
        assert_eq!(a.fog, Some(crate::args::Fog::Off), "{bad}");
    }
    std::fs::write(&path, "[display]\nvsync = true\n").unwrap();
    crate::settings::save_ui_zoom(&path, 200).unwrap();
    let kept = parse_args(words(&["--conf", conf])).unwrap();
    assert_eq!(kept.vsync, Some(Vsync::On));

    use crate::args::HdrMode;
    assert_eq!(
        crate::startup::hdr_wanted(&none),
        (HdrMode::Auto, "the default")
    );
    for (text, mode) in [
        ("\"auto\"", HdrMode::Auto),
        ("\"on\"", HdrMode::On),
        ("\"off\"", HdrMode::Off),
        ("true", HdrMode::On),
        ("false", HdrMode::Off),
    ] {
        let a = read(&format!("[display]\nhdr = {text}\n"), &[]);
        assert_eq!(
            crate::startup::hdr_wanted(&a),
            (mode, "the settings file"),
            "{text}"
        );
    }
    let hdr_flag = read("[display]\nhdr = \"off\"\n", &["--hdr", "on"]);
    assert_eq!(
        crate::startup::hdr_wanted(&hdr_flag),
        (HdrMode::On, "--hdr")
    );
    let hdr_bad = read("[display]\nhdr = \"sdr\"\nfog = \"off\"\n", &[]);
    assert_eq!(hdr_bad.hdr, None);
    assert_eq!(hdr_bad.fog, Some(crate::args::Fog::Off));
    std::fs::remove_dir_all(&dir).unwrap();
}
