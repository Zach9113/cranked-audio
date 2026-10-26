fn main() {
    slint_build::compile("ui/main.slint").expect("ui/main.slint");
    // the icon inside the Windows .exe (nothing happens for other targets)
    println!("cargo:rerun-if-changed=../../data/cranked-audio.ico");
    embed_resource::compile("../../data/cranked-audio.rc", embed_resource::NONE).manifest_optional().unwrap();
}
