use std::env::var;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

/// The exe's icon and the details shown in its Properties window. It's written here so the version
/// always matches Cargo.toml, because quoted text can't be passed to the resource compiler reliably.
fn resource_script(icon: &str) -> String {
    let (major, minor, patch) = (
        var("CARGO_PKG_VERSION_MAJOR").unwrap(),
        var("CARGO_PKG_VERSION_MINOR").unwrap(),
        var("CARGO_PKG_VERSION_PATCH").unwrap(),
    );
    let version = var("CARGO_PKG_VERSION").unwrap();
    let mut script = String::from("#include <winver.h>\n");
    let _ = writeln!(script, "1 ICON \"{}\"", icon.replace('\\', "/"));
    let _ = writeln!(script, "VS_VERSION_INFO VERSIONINFO");
    let _ = writeln!(script, "FILEVERSION {major},{minor},{patch},0");
    let _ = writeln!(script, "PRODUCTVERSION {major},{minor},{patch},0");
    script.push_str(
        "FILEFLAGSMASK VS_FFI_FILEFLAGSMASK\nFILEFLAGS 0\nFILEOS VOS_NT_WINDOWS32\nFILETYPE VFT_APP\n\
         FILESUBTYPE VFT2_UNKNOWN\nBEGIN\nBLOCK \"StringFileInfo\"\nBEGIN\nBLOCK \"040904B0\"\nBEGIN\n",
    );
    for (name, value) in [
        ("FileDescription", "dl-rs Steam relay picker"),
        ("FileVersion", version.as_str()),
        ("InternalName", "dl-rs"),
        ("OriginalFilename", "dl-rs.exe"),
        ("ProductName", "dl-rs"),
        ("ProductVersion", version.as_str()),
    ] {
        let _ = writeln!(script, "VALUE \"{name}\", \"{value}\"");
    }
    script.push_str(
        "END\nEND\nBLOCK \"VarFileInfo\"\nBEGIN\nVALUE \"Translation\", 0x409, 1200\nEND\nEND\n",
    );
    script
}

fn main() {
    let dir = var("CARGO_MANIFEST_DIR").unwrap();
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=app.manifest");
    println!("cargo:rerun-if-changed=assets/dl-rs.ico");
    println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg-bins=/MANIFESTUAC:level='requireAdministrator'");
    println!("cargo:rustc-link-arg-bins=/MANIFESTINPUT:{dir}/app.manifest");
    println!("cargo:rustc-link-arg-bins=/SUBSYSTEM:WINDOWS,10.0");
    println!("cargo:rustc-link-arg-bins=/OSVERSION:10.0");

    let script = Path::new(&var("OUT_DIR").unwrap()).join("dl-rs.rc");
    let icon = Path::new(&dir).join("assets").join("dl-rs.ico");
    fs::write(&script, resource_script(&icon.display().to_string())).unwrap();
    embed_resource::compile(&script, embed_resource::NONE)
        .manifest_optional()
        .unwrap();
}
