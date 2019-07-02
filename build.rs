use std::env;
use std::fs::File;
use std::io::Write;
use std::path::Path;

fn main() {
    let out_dir = env::var("OUT_DIR").expect("no out directory");
    let dest = Path::new(&out_dir).join("build_constants.rs");

    let mut file = File::create(&dest).expect("could not create file");

    let freq: u32 = option_env!("HAZPTR_SCAN_FREQ")
        .map_or(Ok(100), str::parse)
        .expect("failed to parse env variable HAZPTR_SCAN_FREQ");

    if freq == 0 {
        panic!("invalid HAZPTR_RECLAMATION_FREQ value (0)");
    }

    write!(&mut file, "const SCAN_THRESHOLD: u32 = {};", freq).expect("could not write to file");
}
