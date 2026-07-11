//! Platform link adjustments for native dependencies.

use std::path::Path;

fn main() {
    // lbug's current macOS prebuilt static archive references OpenSSL but
    // does not propagate its link directives. Homebrew provides the library
    // outside the default linker search path on Apple Silicon; emit both the
    // search path (when present) and the two dynamic libraries for the final
    // application/test link. Other platforms already discover OpenSSL through
    // their standard linker configuration.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    const HOMEBREW_OPENSSL: &str = "/opt/homebrew/opt/openssl@3/lib";
    if Path::new(HOMEBREW_OPENSSL).is_dir() {
        println!("cargo:rustc-link-search=native={HOMEBREW_OPENSSL}");
    }
    println!("cargo:rustc-link-lib=dylib=ssl");
    println!("cargo:rustc-link-lib=dylib=crypto");
}
