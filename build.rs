//! Platform link adjustments for native dependencies.

use std::path::Path;

mod build_support;

fn main() {
    let Ok(target_os) = std::env::var("CARGO_CFG_TARGET_OS") else {
        return;
    };
    let plan = build_support::native_link_plan(&target_os);

    // lbug's prebuilt static archives reference OpenSSL but do not propagate
    // its link directives. Linux finds OpenSSL through its standard linker
    // configuration. Homebrew installs it outside the default search path on
    // Apple Silicon, so macOS additionally probes that well-known location.
    const HOMEBREW_OPENSSL: &str = "/opt/homebrew/opt/openssl@3/lib";
    if plan.probe_homebrew_openssl && Path::new(HOMEBREW_OPENSSL).is_dir() {
        println!("cargo:rustc-link-search=native={HOMEBREW_OPENSSL}");
    }
    if plan.link_openssl {
        println!("cargo:rustc-link-lib=dylib=ssl");
        println!("cargo:rustc-link-lib=dylib=crypto");
    }
}
