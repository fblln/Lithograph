//! Rust helper library for the polyglot fixture.

use std::env;

/// Provides route baking behavior.
pub trait RouteBake {
    /// Returns a short route summary.
    fn bake(&self, route: &str) -> String;
}

/// Configurable route baker.
pub struct RouteBaker {
    cache_dir: String,
}

impl RouteBaker {
    /// Builds a baker from environment configuration.
    pub fn from_env() -> Self {
        let cache_dir = env::var("RIDGELINE_CACHE_DIR").unwrap_or_else(|_| "target/cache".to_owned());
        Self { cache_dir }
    }
}

impl RouteBake for RouteBaker {
    fn bake(&self, route: &str) -> String {
        format!("baked:{route}:{}", self.cache_dir)
    }
}

/// Bakes a route with the default baker.
pub fn bake_route(route: &str) -> String {
    RouteBaker::from_env().bake(route)
}

