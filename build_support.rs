//! Deterministic native-link decisions shared by the build script and tests.

/// Native libraries and platform-specific search paths required by `lbug`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NativeLinkPlan {
    /// Whether the final binary must link OpenSSL's `ssl` and `crypto` libraries.
    pub(crate) link_openssl: bool,
    /// Whether to probe the Apple Silicon Homebrew OpenSSL search path.
    pub(crate) probe_homebrew_openssl: bool,
}

/// Computes native-link requirements from Cargo's target operating-system name.
pub(crate) const fn native_link_plan(target_os: &str) -> NativeLinkPlan {
    match target_os.as_bytes() {
        b"macos" => NativeLinkPlan {
            link_openssl: true,
            probe_homebrew_openssl: true,
        },
        b"linux" => NativeLinkPlan {
            link_openssl: true,
            probe_homebrew_openssl: false,
        },
        _ => NativeLinkPlan {
            link_openssl: false,
            probe_homebrew_openssl: false,
        },
    }
}
