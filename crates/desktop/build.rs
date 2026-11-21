use std::{env, path::PathBuf, process::Command};

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let icons =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../../packaging/icons");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    println!("cargo:rerun-if-changed={}", icons.join("app.rc").display());
    println!("cargo:rerun-if-changed={}", icons.join("app.ico").display());

    let target = env::var("TARGET").unwrap();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap();
    let (mut compiler, resource) = if target_env == "msvc" {
        let resource = out.join("app-icon.res");
        let mut rc = cc::windows_registry::find(&target, "rc.exe")
            .expect("Windows SDK resource compiler (rc.exe) is required");
        rc.arg("/nologo").arg("/fo").arg(&resource).arg("app.rc");
        (rc, resource)
    } else {
        println!("cargo:rerun-if-env-changed=WINDRES");
        let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();
        let prefix = if arch == "x86" { "i686" } else { &arch };
        let windres = env::var_os("WINDRES").unwrap_or_else(|| {
            if env::var("HOST").as_deref() == Ok(&target) {
                "windres".into()
            } else {
                format!("{prefix}-w64-mingw32-windres").into()
            }
        });
        let resource = out.join("app-icon.o");
        let mut rc = Command::new(windres);
        rc.args(["-O", "coff", "-i", "app.rc", "-o"]).arg(&resource);
        (rc, resource)
    };

    let status = compiler
        .current_dir(&icons)
        .status()
        .unwrap_or_else(|error| panic!("Could not run {compiler:?}: {error}"));
    assert!(status.success(), "{compiler:?} failed: {status}");
    println!("cargo:rustc-link-arg-bins={}", resource.display());
}
