use std::env;
use std::fs::File;
use std::io::Write;
use std::path::Path;

//HAZPTR_SCAN_THRESHOLD

fn main() {
    println!("cargo:rerun-if-env-changed=HAZPTR_SCAN_THRESHOLD");

    let out_dir = env::var("OUT_DIR").expect("no out directory");
    let dest = Path::new(&out_dir).join("build_constants.rs");

    let mut file = File::create(&dest).expect("could not create file");

    let scan: u32 = option_env!("HAZPTR_SCAN_THRESHOLD")
        .map_or(Ok(100), str::parse)
        .expect("failed to parse env variable HAZPTR_SCAN_THRESHOLD");

    if scan == 0 {
        panic!("invalid HAZPTR_SCAN_THRESHOLD value (0)");
    }

    write!(&mut file, "const SCAN_THRESHOLD: u32 = {};", scan).expect("could not write to file");
}
