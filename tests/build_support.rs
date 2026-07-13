//! Regression tests for native dependency link decisions.

#[path = "../build_support.rs"]
mod build_support;

use build_support::{NativeLinkPlan, native_link_plan};

#[test]
fn linux_links_openssl_from_the_standard_search_path() {
    assert_eq!(
        native_link_plan("linux"),
        NativeLinkPlan {
            link_openssl: true,
            probe_homebrew_openssl: false,
        }
    );
}

#[test]
fn macos_links_openssl_and_probes_homebrew() {
    assert_eq!(
        native_link_plan("macos"),
        NativeLinkPlan {
            link_openssl: true,
            probe_homebrew_openssl: true,
        }
    );
}

#[test]
fn unsupported_targets_do_not_receive_unverified_link_directives() {
    for target_os in ["windows", "freebsd", "android", ""] {
        assert_eq!(
            native_link_plan(target_os),
            NativeLinkPlan {
                link_openssl: false,
                probe_homebrew_openssl: false,
            },
            "unexpected link plan for {target_os}"
        );
    }
}
