//! `golden`, `quality`, and `validate-mermaid`: deterministic checks against
//! already-generated documentation. None of these call a language model.

use crate::cli::{GoldenArgs, OutputFormat, QualityArgs, ValidateMermaidArgs};
use crate::golden::{check_or_update, render_report as render_golden_report};
use crate::mermaid::{
    fix_path as fix_mermaid_path, render_report as render_mermaid_report,
    validate as validate_mermaid,
};
use crate::quality::{inspect as inspect_quality, render_table as render_quality_table};
use std::io::Write;

pub(crate) fn execute_golden<W>(
    args: GoldenArgs,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let report = check_or_update(&args.path, &args.golden_dir, args.update)?;
    writer.write_all(render_golden_report(&report).as_bytes())?;
    Ok(())
}

pub(crate) fn execute_quality<W>(
    args: QualityArgs,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let report = inspect_quality(&args.path)?;
    let output = match args.format {
        OutputFormat::Table => render_quality_table(&report),
        OutputFormat::Json => {
            let mut json = serde_json::to_string_pretty(&report)?;
            json.push('\n');
            json
        }
    };
    writer.write_all(output.as_bytes())?;
    if report.is_clean() {
        Ok(())
    } else {
        Err("quality inspection failed".into())
    }
}

pub(crate) fn execute_validate_mermaid<W>(
    args: ValidateMermaidArgs,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    if args.fix {
        let files_changed = fix_mermaid_path(&args.path)?;
        writer.write_all(format!("fixed node ids in {files_changed} file(s)\n").as_bytes())?;
    }
    let report = validate_mermaid(&args.path, args.node_validator.as_deref())?;
    writer.write_all(render_mermaid_report(&report).as_bytes())?;
    if report.is_clean() {
        Ok(())
    } else {
        Err("Mermaid validation failed".into())
    }
}
