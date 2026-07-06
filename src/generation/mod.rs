//! Documentation generation: the model boundary, prompt/context building,
//! rendering, and evidence validation.

pub mod context;
pub mod llm;
pub mod openai;
pub mod render;
pub mod validate;

pub use context::{ContextBuilder, ContextExcerpt, ModelContext};
pub use llm::{LanguageModel, MockModel, ModelError, ModelRequest, PageGeneration};
pub use openai::{OpenAiConfig, OpenAiModel};
pub use render::{PageRenderer, PageWriteOutcome, RenderError};
pub use validate::{EvidenceIssue, EvidenceValidator};
