//! Retrieval surfaces over the graph and generated docs: full-text search,
//! deterministic semantic (embedding) search, the FTS index, and the local
//! question-answering query layer.

pub(crate) mod chunk_enrich;
pub(crate) mod chunk_index;
pub(crate) mod chunk_rank;
pub(crate) mod code_index;
pub(crate) mod embedding_profile;
pub(crate) mod fts;
pub(crate) mod fts_incremental;
#[cfg(test)]
mod hybrid_gate;
pub(crate) mod query;
pub(crate) mod search;
pub(crate) mod semantic_search;
