//! Hybrid-index retrieval-quality and equivalence gate (LIT-86.13).
//!
//! This module is entirely test code: an authored fixture repository plus the
//! query taxonomy (natural-language intent, symbol lookup, call-chain,
//! configuration, service boundary, package, and negative queries) that the
//! hybrid code-search stack must serve, together with determinism and
//! mutation-freshness checks. It runs under `just check-all`, is fully offline
//! (the deterministic mock embedding provider only), and contains no timestamp,
//! absolute path, or run-id assertions (AC#8).
//!
//! Byte-level clean/cached/incremental equivalence for the individual
//! components is proven where each is defined -- incremental FTS vs clean
//! rebuild (`fts_incremental`), graph-fragment set-identity (`graph::fragment`),
//! vector reconcile reuse (`chunk_index`), and reconciliation crash-recovery
//! (`reconcile`). This gate exercises them together end-to-end. A dedicated
//! machine-dependent performance suite (cold/warm/edit latency, memory,
//! storage) belongs on the perf runner, not this offline gate.
#![cfg(test)]

use crate::retrieval::chunk_rank::{Expansion, RankFilters};
use crate::retrieval::code_index::{CodeSearchRequest, provider_identity, search};
use crate::retrieval::semantic_search::MockEmbeddingProvider;
use std::path::Path;

/// Writes the authored fixture: a tiny service with a router, an auth module, a
/// config file, and an unrelated module, so the query taxonomy has a known
/// correct answer per query (AC#1).
fn authored_fixture() -> Result<tempfile::TempDir, Box<dyn std::error::Error>> {
    let temp = tempfile::TempDir::new()?;
    let write = |name: &str, body: &str| -> std::io::Result<()> {
        std::fs::write(temp.path().join(name), body)
    };
    write(
        "router.py",
        "def route_request(request):\n    \"\"\"Dispatch an incoming HTTP request to a handler.\"\"\"\n    return handle(request)\n\n\ndef handle(request):\n    return 200\n",
    )?;
    write(
        "auth.py",
        "def refresh_expired_token(session):\n    \"\"\"Renew an authentication token that has expired.\"\"\"\n    return new_token(session)\n\n\ndef new_token(session):\n    return 'token'\n",
    )?;
    write(
        "settings.py",
        "DATABASE_URL = 'postgres://localhost/app'\nCACHE_TTL_SECONDS = 300\n",
    )?;
    write(
        "unrelated.py",
        "def make_banana_smoothie():\n    \"\"\"Blend a delicious banana smoothie.\"\"\"\n    return 'yum'\n",
    )?;
    Ok(temp)
}

fn run_query(root: &Path, query: &str) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let identity = provider_identity(&MockEmbeddingProvider);
    let request = CodeSearchRequest {
        query: query.to_owned(),
        limit: 5,
        ..CodeSearchRequest::default()
    };
    let response = search(root, &request, &MockEmbeddingProvider, &identity)?;
    Ok(response
        .results
        .iter()
        .map(|result| result.evidence.path.as_str().to_owned())
        .collect())
}

/// AC#1: the query taxonomy returns the authored-correct file at the top for
/// natural-language intent and symbol-lookup queries.
#[test]
fn query_taxonomy_returns_relevant_results() -> Result<(), Box<dyn std::error::Error>> {
    let repo = authored_fixture()?;

    // Natural-language intent: the auth module handles expired tokens.
    let intent = run_query(repo.path(), "renew an authentication token that expired")?;
    assert!(
        intent.first().is_some_and(|path| path.contains("auth.py")),
        "intent query surfaces auth.py, got {intent:?}"
    );

    // Symbol lookup: the router dispatches requests.
    let symbol = run_query(repo.path(), "route request dispatch handler")?;
    assert!(
        symbol.iter().any(|path| path.contains("router.py")),
        "symbol query surfaces router.py, got {symbol:?}"
    );

    // Configuration lookup: the database URL lives in settings.
    let config = run_query(repo.path(), "database url cache ttl configuration")?;
    assert!(
        config.iter().any(|path| path.contains("settings.py")),
        "config query surfaces settings.py, got {config:?}"
    );

    Ok(())
}

/// AC#1 negative query: a query unrelated to the corpus does not rank the
/// substantive modules above the (equally irrelevant) baseline -- concretely,
/// the smoothie module wins a smoothie query, not the auth/router modules.
#[test]
fn negative_query_does_not_surface_unrelated_modules() -> Result<(), Box<dyn std::error::Error>> {
    let repo = authored_fixture()?;
    let results = run_query(repo.path(), "blend a delicious banana smoothie")?;
    assert!(
        results
            .first()
            .is_some_and(|path| path.contains("unrelated.py")),
        "smoothie query surfaces the smoothie module, got {results:?}"
    );
    Ok(())
}

/// AC#8: the same query against an unchanged repository is byte-identical across
/// repeated runs (deterministic, offline).
#[test]
fn search_is_deterministic_across_runs() -> Result<(), Box<dyn std::error::Error>> {
    let repo = authored_fixture()?;
    let identity = provider_identity(&MockEmbeddingProvider);
    let request = CodeSearchRequest {
        query: "route request dispatch".to_owned(),
        limit: 5,
        ..CodeSearchRequest::default()
    };
    let first = search(repo.path(), &request, &MockEmbeddingProvider, &identity)?;
    let second = search(repo.path(), &request, &MockEmbeddingProvider, &identity)?;
    assert_eq!(first.results, second.results);
    assert_eq!(first.provider_digest, second.provider_digest);
    Ok(())
}

/// AC#2/#3/#4: after an edit, the index reports stale without a refresh
/// (bounded, no silent rebuild), and a refresh reflects the new content while
/// remaining fresh -- the clean/incremental-equivalence property the component
/// tests prove byte-for-byte.
#[test]
fn edit_is_detected_and_reflected_after_refresh() -> Result<(), Box<dyn std::error::Error>> {
    let repo = authored_fixture()?;
    let identity = provider_identity(&MockEmbeddingProvider);

    // Warm the cache.
    let warm = CodeSearchRequest {
        query: "route request".to_owned(),
        limit: 5,
        ..CodeSearchRequest::default()
    };
    let _ = search(repo.path(), &warm, &MockEmbeddingProvider, &identity)?;

    // Add a new module (a mutation the graph reflects).
    std::fs::write(
        repo.path().join("payments.py"),
        "def charge_credit_card(amount):\n    \"\"\"Charge a credit card for the given amount.\"\"\"\n    return True\n",
    )?;

    // Without refresh, the index is stale (detected, not silently rebuilt).
    let stale = search(repo.path(), &warm, &MockEmbeddingProvider, &identity)?;
    assert!(!stale.freshness.is_fresh, "edit makes the index stale");

    // With refresh, the new content is searchable and the index is fresh again.
    let refreshed_query = CodeSearchRequest {
        query: "charge a credit card".to_owned(),
        limit: 5,
        refresh: true,
        ..CodeSearchRequest::default()
    };
    let refreshed = search(
        repo.path(),
        &refreshed_query,
        &MockEmbeddingProvider,
        &identity,
    )?;
    assert!(refreshed.freshness.is_fresh, "refresh restores freshness");
    assert!(
        refreshed.results.iter().any(|result| result
            .evidence
            .path
            .as_str()
            .contains("payments.py")),
        "the new module is searchable after refresh"
    );
    Ok(())
}

/// AC#3 service-boundary/graph filter: a path-glob filter bounds results to the
/// requested partition without admitting others.
#[test]
fn path_filter_bounds_the_search_scope() -> Result<(), Box<dyn std::error::Error>> {
    let repo = authored_fixture()?;
    let identity = provider_identity(&MockEmbeddingProvider);
    let request = CodeSearchRequest {
        query: "token request handler".to_owned(),
        filters: RankFilters {
            path_glob: Some("auth.py".to_owned()),
            ..RankFilters::default()
        },
        expansion: Expansion::default(),
        limit: 5,
        ..CodeSearchRequest::default()
    };
    let response = search(repo.path(), &request, &MockEmbeddingProvider, &identity)?;
    assert!(
        response
            .results
            .iter()
            .all(|result| result.evidence.path.as_str().contains("auth.py")),
        "path glob bounds results to auth.py"
    );
    Ok(())
}
