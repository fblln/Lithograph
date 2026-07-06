//! Manual smoke test for the OpenAI-compatible adapter.
//!
//! Not run by `cargo test` — it needs a real key or a local OpenAI-compatible
//! server (Ollama, LM Studio, vLLM, etc.). Run explicitly:
//!
//! ```sh
//! LITHOGRAPH_OPENAI_API_KEY=sk-... \
//! LITHOGRAPH_OPENAI_MODEL=gpt-4o-mini \
//! cargo run --example openai_smoke
//! ```
//!
//! Point at a local server instead of the real API with:
//!
//! ```sh
//! LITHOGRAPH_OPENAI_BASE_URL=http://localhost:11434/v1 \
//! LITHOGRAPH_OPENAI_MODEL=llama3 \
//! LITHOGRAPH_OPENAI_API_KEY=unused \
//! cargo run --example openai_smoke
//! ```

use lithograph::generation::{LanguageModel, ModelRequest, OpenAiConfig, OpenAiModel};
use lithograph::manifest::TaskKind;
use std::io::Write;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let base_url = std::env::var("LITHOGRAPH_OPENAI_BASE_URL")
        .unwrap_or_else(|_| "https://api.openai.com/v1".to_owned());
    let api_key = std::env::var("LITHOGRAPH_OPENAI_API_KEY")?;
    let model =
        std::env::var("LITHOGRAPH_OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_owned());

    let adapter = OpenAiModel::new(OpenAiConfig::new(base_url, api_key, model.clone()));
    let request = ModelRequest {
        model,
        prompt_version: "smoke-v1".to_owned(),
        task_kind: TaskKind::ModulePage,
        input_hash: "smoke-test".to_owned(),
        system_prompt: "You are a terse assistant.".to_owned(),
        user_prompt: "Reply with exactly the word: pong".to_owned(),
    };

    let text = adapter.generate_text(&request)?;
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "model replied: {text}")?;
    Ok(())
}
