//! `init`, `update`, and `watch`: scan/analyze/plan/generate documentation,
//! selectively regenerate it, and poll a repository for staleness.

use crate::cli::{InitArgs, WatchArgs};
use crate::generation::{
    DeepInfraConfig, DeepInfraModel, LanguageModel, MockModel, OpenAiConfig, OpenAiModel,
};
use crate::orchestrate::{
    InitReport, UpdateReport, run_init_with_options, run_update, run_update_with_options,
};
use crate::watch::{WatchConfig, poll_once, render_report as render_watch_report};
use std::io::Write;
use std::time::Duration;

pub(crate) fn execute_init<W>(
    args: InitArgs,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let (model, model_name) = select_model()?;
    let report = run_init_with_options(
        &args.path,
        model.as_ref(),
        &model_name,
        &args.prompt_version,
        args.semantic_grouping,
    )?;
    writer.write_all(render_init_report(&report).as_bytes())?;
    Ok(())
}

pub(crate) fn execute_update<W>(
    args: InitArgs,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let (model, model_name) = select_model()?;
    let report = run_update_with_options(
        &args.path,
        model.as_ref(),
        &model_name,
        &args.prompt_version,
        args.semantic_grouping,
    )?;
    writer.write_all(render_update_report(&report).as_bytes())?;
    Ok(())
}

/// Selects a language model backend from environment variables, in order:
/// DeepInfra (via `rig-core`) when `LITHOGRAPH_DEEPINFRA_API_KEY` is set,
/// then the direct OpenAI-compatible adapter when `LITHOGRAPH_OPENAI_API_KEY`
/// is set, otherwise the deterministic mock (the zero-configuration default
/// so `init` always works without credentials).
pub(crate) fn select_model() -> Result<(Box<dyn LanguageModel>, String), Box<dyn std::error::Error>>
{
    if let Ok(api_key) = std::env::var("LITHOGRAPH_DEEPINFRA_API_KEY") {
        let model_name = std::env::var("LITHOGRAPH_DEEPINFRA_MODEL").map_err(|_| {
            "LITHOGRAPH_DEEPINFRA_API_KEY is set but LITHOGRAPH_DEEPINFRA_MODEL is not; \
             set it to the DeepInfra model path to use (e.g. a DeepSeek model)"
        })?;
        let mut config = DeepInfraConfig::new(api_key, model_name.clone());
        if let Ok(base_url) = std::env::var("LITHOGRAPH_DEEPINFRA_BASE_URL") {
            config = config.with_base_url(base_url);
        }
        if let Ok(reasoning_effort) = std::env::var("LITHOGRAPH_DEEPINFRA_REASONING_EFFORT") {
            config = config.with_reasoning_effort(reasoning_effort);
        }
        return Ok((Box::new(DeepInfraModel::new(config)?), model_name));
    }

    match std::env::var("LITHOGRAPH_OPENAI_API_KEY") {
        Ok(api_key) => {
            let base_url = std::env::var("LITHOGRAPH_OPENAI_BASE_URL")
                .unwrap_or_else(|_| "https://api.openai.com/v1".to_owned());
            let model_name = std::env::var("LITHOGRAPH_OPENAI_MODEL")
                .unwrap_or_else(|_| "gpt-4o-mini".to_owned());
            let mut config = OpenAiConfig::new(base_url, api_key, model_name.clone());
            if let Ok(reasoning_effort) = std::env::var("LITHOGRAPH_OPENAI_REASONING_EFFORT") {
                config = config.with_reasoning_effort(reasoning_effort);
            }
            Ok((Box::new(OpenAiModel::new(config)), model_name))
        }
        Err(_) => Ok((Box::new(MockModel), "mock".to_owned())),
    }
}

/// Renders a deterministic, human-readable `init` summary.
fn render_init_report(report: &InitReport) -> String {
    format!(
        "artifacts: {}\n\
         graph nodes: {}\n\
         graph relations: {}\n\
         modules: {}\n\
         pages: {}\n\
         pages written: {}\n\
         changed artifacts: {}\n\
         reanalyzed artifacts: {}\n\
         graph: {}\n\
         manifest: {}\n\
         run metadata: {}\n",
        report.artifact_count,
        report.graph_node_count,
        report.graph_relation_count,
        report.module_count,
        report.page_count,
        report.pages_written,
        report.changed_artifact_count,
        report.artifacts_reanalyzed_count,
        report.graph_path.display(),
        report.manifest_path.display(),
        report.run_metadata_path.display(),
    )
}

/// Renders a deterministic, human-readable `update` summary.
fn render_update_report(report: &UpdateReport) -> String {
    format!(
        "artifacts: {}\n\
         graph nodes: {}\n\
         graph relations: {}\n\
         modules: {}\n\
         pages: {}\n\
         pages regenerated: {}\n\
         changed artifacts: {}\n\
         reanalyzed artifacts: {}\n\
         graph: {}\n\
         manifest: {}\n\
         run metadata: {}\n",
        report.artifact_count,
        report.graph_node_count,
        report.graph_relation_count,
        report.module_count,
        report.page_count,
        report.pages_regenerated,
        report.changed_artifact_count,
        report.artifacts_reanalyzed_count,
        report.graph_path.display(),
        report.manifest_path.display(),
        report.run_metadata_path.display(),
    )
}

/// Polls `args.path` for staleness and, with `--auto-index`, runs `update`
/// when staleness is detected (AC1: never auto-indexes without that
/// explicit flag). `--once` polls a single time and returns; otherwise this
/// loops with a real `std::thread::sleep` between polls, which is why only
/// the single-poll (`--once`) path is unit-tested -- an infinite loop with
/// real sleeps has no deterministic endpoint to assert against.
pub(crate) fn execute_watch<W>(
    args: WatchArgs,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let config = WatchConfig {
        max_artifacts: args.max_artifacts,
        poll_interval: Duration::from_secs(args.interval_secs),
    };

    loop {
        let report = poll_once(&args.path, &config)?;
        writer.write_all(render_watch_report(&report).as_bytes())?;
        if report.stale && args.auto_index {
            let (model, model_name) = select_model()?;
            let update_report = run_update(&args.path, model.as_ref(), &model_name, "v1")?;
            writer.write_all(render_update_report(&update_report).as_bytes())?;
        }
        if args.once {
            return Ok(());
        }
        std::thread::sleep(config.poll_interval);
    }
}
