//! ONNX Runtime's CoreML code calls ___isPlatformVersionAtLeast, which lives in
//! clang's runtime library; Rust links with -nodefaultlibs, so add it.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        let out = std::process::Command::new("xcrun").args(["clang", "--print-runtime-dir"]).output();
        if let Ok(o) = out {
            let dir = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if !dir.is_empty() {
                println!("cargo:rustc-link-search=native={dir}");
                println!("cargo:rustc-link-lib=static=clang_rt.osx");
            }
        }
    }
}
