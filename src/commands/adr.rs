//! `adr`: create, read, update, delete, and list architecture decision
//! records.

use crate::cli::{
    AdrCommand, AdrCreateArgs, AdrDeleteArgs, AdrGetArgs, AdrListArgs, AdrTarget, AdrUpdateArgs,
    OutputFormat,
};
use crate::docs::adr::{AdrRecord, AdrStore, AdrSummary};
use std::io::Write;

pub(crate) fn execute_adr<W>(
    command: AdrCommand,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    match command.target {
        AdrTarget::Create(args) => execute_adr_create(args, writer),
        AdrTarget::Get(args) => execute_adr_get(args, writer),
        AdrTarget::Update(args) => execute_adr_update(args, writer),
        AdrTarget::Delete(args) => execute_adr_delete(args, writer),
        AdrTarget::List(args) => execute_adr_list(args, writer),
    }
}

fn execute_adr_create<W>(
    args: AdrCreateArgs,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let record = AdrStore::new(&args.path).create(
        &args.title,
        &args.context,
        &args.decision,
        args.consequences.as_deref(),
    )?;
    write_adr_record(writer, &record, args.format)
}

fn execute_adr_get<W>(args: AdrGetArgs, writer: &mut W) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let record = AdrStore::new(&args.path).get(&args.id)?;
    write_adr_record(writer, &record, args.format)
}

fn execute_adr_update<W>(
    args: AdrUpdateArgs,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let store = AdrStore::new(&args.path);
    let mut record = store.get(&args.id)?;
    if let (Some(section), Some(value)) = (&args.section, &args.value) {
        record = store.update_section(&args.id, section, value)?;
    }
    if let Some(status) = args.status {
        record = store.update_status(&args.id, status.into())?;
    }
    write_adr_record(writer, &record, args.format)
}

fn execute_adr_delete<W>(
    args: AdrDeleteArgs,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    AdrStore::new(&args.path).delete(&args.id)?;
    writer.write_all(format!("deleted {}\n", args.id).as_bytes())?;
    Ok(())
}

fn execute_adr_list<W>(args: AdrListArgs, writer: &mut W) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let summaries = AdrStore::new(&args.path).list();
    let output = match args.format {
        OutputFormat::Table => render_adr_list_table(&summaries),
        OutputFormat::Json => {
            let mut json = serde_json::to_string_pretty(&summaries)?;
            json.push('\n');
            json
        }
    };
    writer.write_all(output.as_bytes())?;
    Ok(())
}

fn write_adr_record<W>(
    writer: &mut W,
    record: &AdrRecord,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let output = match format {
        OutputFormat::Table => render_adr_record_table(record),
        OutputFormat::Json => {
            let mut json = serde_json::to_string_pretty(record)?;
            json.push('\n');
            json
        }
    };
    writer.write_all(output.as_bytes())?;
    Ok(())
}

/// Renders one ADR as a deterministic, human-readable table.
fn render_adr_record_table(record: &AdrRecord) -> String {
    let mut output = format!("{} [{:?}] {}\n", record.id, record.status, record.title);
    for (section, content) in &record.sections {
        output.push_str(&format!("- {section}: {content}\n"));
    }
    output
}

/// Renders every ADR as a deterministic, human-readable table.
fn render_adr_list_table(summaries: &[AdrSummary]) -> String {
    if summaries.is_empty() {
        return "no ADRs recorded\n".to_owned();
    }
    let mut output = format!("{} ADR(s):\n", summaries.len());
    for summary in summaries {
        output.push_str(&format!(
            "{} [{:?}] {}\n",
            summary.id, summary.status, summary.title
        ));
    }
    output
}
