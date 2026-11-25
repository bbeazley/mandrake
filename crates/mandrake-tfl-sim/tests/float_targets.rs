use mandrake_tfl_sim::trig::float_target_hashes;

const TABLES: u64 = 0x2a21_db74_9426_fcde;

const RAW_GLIBC: [u64; 2] = [0x1790_6f08_300d_c8df, 0x89d5_9ae8_047b_13f2];
const RAW_WASM: [u64; 2] = [0x1790_6f08_300d_c8df, 0xf5bb_2671_d61d_42cd];

fn get(name: &str) -> u64 {
    float_target_hashes()
        .into_iter()
        .find(|(n, _)| *n == name)
        .unwrap()
        .1
}

#[test]
fn tfl_float_hashes() {
    for (n, h) in float_target_hashes() {
        println!("{n}\t{h:016x}");
    }
    assert_eq!(get("tfl.trig.tables"), TABLES, "tfl.trig.tables");
    let raw = [get("tfl.atan1.raw"), get("tfl.trig.sin_cos.raw")];
    if cfg!(all(target_os = "linux", target_env = "gnu")) {
        assert_eq!(raw, RAW_GLIBC, "glibc's sin, cos and atan changed");
    } else if cfg!(target_arch = "wasm32") {
        assert_eq!(raw, RAW_WASM, "wasm's sin, cos and atan changed");
    }
}

thread_local! {
    static HASHES: std::cell::RefCell<Vec<(&'static str, u64)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}
