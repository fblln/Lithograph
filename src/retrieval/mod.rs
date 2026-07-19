//! Retrieval surfaces over the graph and generated docs: full-text search,
//! deterministic semantic (embedding) search, the FTS index, and the local
//! question-answering query layer.

pub(crate) mod fts;
pub(crate) mod query;
pub(crate) mod search;
pub(crate) mod semantic_search;
