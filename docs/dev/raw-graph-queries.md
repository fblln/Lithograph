# Raw graph query access

Use Lithograph's typed query and MCP APIs for schema, search, tracing,
architecture, impact, semantic search, and statistics. Raw Ladybug/Cypher
queries are disabled by default and may only be enabled by a trusted local
caller through `RawQueryAccess::trusted()`. Never pass user-supplied raw
queries through that guard.
