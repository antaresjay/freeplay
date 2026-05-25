fn main() {
    // The icon is compiled into the exe as a windows resource by the line
    // below, but cargo will not rerun this script when only the icon changes,
    // so a rebuild quietly keeps embedding the previous one.
    println!("cargo:rerun-if-changed=icons");
    tauri_build::build()
}
