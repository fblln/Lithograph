use crate::cli::{ResearchCommand, ResearchTarget};
use crate::graph::GraphStore;
use crate::knowledge::research_feedback::{
    AnswerResultInput, ResearchFeedbackStore, unix_timestamp_now,
};
use std::io::Write;

pub(super) fn execute_research<W>(
    command: ResearchCommand,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    match command.target {
        ResearchTarget::SaveResult(args) => {
            let recorded_at = args.recorded_at.map_or_else(unix_timestamp_now, Ok)?;
            let result = ResearchFeedbackStore::new(&args.path).save_result(AnswerResultInput {
                question: args.question,
                answer: args.answer,
                cited_node_ids: args.cited_node_ids,
                outcome: args.outcome.into(),
                correction: args.correction,
                recorded_at,
            })?;
            writeln!(writer, "{}", serde_json::to_string_pretty(&result)?)?;
        }
        ResearchTarget::Reflect(args) => {
            let now = args.now.map_or_else(unix_timestamp_now, Ok)?;
            let graph = GraphStore::new(&args.path).load()?.graph;
            let lessons = ResearchFeedbackStore::new(&args.path).reflect(&graph, now)?;
            writeln!(writer, "{}", serde_json::to_string_pretty(&lessons)?)?;
        }
    }
    Ok(())
}
