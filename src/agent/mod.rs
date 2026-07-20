//! Agent-facing surfaces: the MCP server and its target integrations, the
//! deterministic knowledge-agent framework, the higher-level agent registry,
//! and the local `ask` question interface.

pub(crate) mod agents;
pub(crate) mod ask;
pub(crate) mod knowledge_agent;
pub(crate) mod mcp;
pub(crate) mod mcp_targets;
