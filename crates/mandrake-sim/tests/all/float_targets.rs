use mandrake_sim::float_targets::hashes;

const STRIDE: u32 = 4099;

enum Pin {
    Same(u64),
    Raw { glibc: u64, wasm: u64 },
}

const PINS: [(&str, Pin); 6] = [
    (
        "engine_move.sin_cos.raw",
        Pin::Raw {
            glibc: 0x89d5_9ae8_047b_13f2,
            wasm: 0xf5bb_2671_d61d_42cd,
        },
    ),
    ("engine_move.tables", Pin::Same(0x3ada_c65b_b567_9e6e)),
    (
        "particles.cos.raw",
        Pin::Raw {
            glibc: 0x199a_fe21_c674_015f,
            wasm: 0xfa21_68ea_3bf5_8bb6,
        },
    ),
    ("particles.cos.steps", Pin::Same(0xbca0_f2de_9905_a08e)),
    (
        "model_hit.sin_cos.raw",
        Pin::Raw {
            glibc: 0x5832_dfe3_47c9_8948,
            wasm: 0x0566_98d2_600b_30de,
        },
    ),
    ("model_hit.rotation", Pin::Same(0x95cd_a737_4a75_6be1)),
];

#[test]
fn library_results_pinned() {
    let got = hashes(STRIDE);
    assert_eq!(got.len(), PINS.len());
    let glibc = cfg!(all(target_os = "linux", target_env = "gnu"));
    let wasm = cfg!(target_arch = "wasm32");
    for ((name, h), (pin, want)) in got.into_iter().zip(PINS) {
        assert_eq!(name, pin);
        match want {
            Pin::Same(v) => assert_eq!(h, v, "{name}: what the sim uses changed"),
            Pin::Raw { glibc: v, .. } if glibc => {
                assert_eq!(h, v, "{name}: glibc's results changed; recheck the margins")
            }
            Pin::Raw { wasm: v, .. } if wasm => {
                assert_eq!(
                    h, v,
                    "{name}: wasm's libm results changed; recheck the margins"
                )
            }
            Pin::Raw { .. } => {}
        }
    }
}
