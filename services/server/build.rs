//! On Windows (MSVC): links the logo icon and the version info into
//! `ocw-server.exe` (`ocw-winres`, spec §11.3); nothing on other targets.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    ocw_winres::embed("Open Cycling World server");
}
