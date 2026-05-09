use serde::{Deserialize, Serialize};
use std::io::Write;
use std::{env, fs, io, path::Path};

#[derive(Serialize, Deserialize, Clone, Debug)]
struct AtCoderLanguage {
    n: String,
    v: u16,
}

type LanguageData = Vec<AtCoderLanguage>;
static RESOURCE_PATH: &str = "src/resources/languages-2025_2026.json";

fn main() {
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR not set");
    let dest_path = Path::new(&out_dir).join("languages-2025_2026.bin.zst");

    println!("cargo:rerun-if-changed={}", RESOURCE_PATH);

    let file = fs::File::open(RESOURCE_PATH).expect("Failed to open file");
    let reader = io::BufReader::new(file);
    let data: LanguageData = serde_json::from_reader(reader).expect("Failed to parse JSON");

    let bin_data = postcard::to_allocvec(&data).expect("Failed to serialize to postcard");

    let mut output = fs::File::create(dest_path).expect("Failed to create output file");

    let mut writer = io::BufWriter::new(&mut output);

    zstd::stream::copy_encode(bin_data.as_slice(), &mut writer, 22)
        .expect("Failed to copy data to zst");

    writer.flush().expect("Failed to flush output");
}
