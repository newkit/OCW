//! On Windows (MSVC): links the studio's logo icon (`artwork/logos/
//! ocw-studio-*.png`) and the version info into `ocw-studio.exe`
//! (`ocw-winres`, spec §11.3); nothing on other targets.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    ocw_winres::embed_logo("OCW Studio", &ocw_winres::OCW_STUDIO);
}
