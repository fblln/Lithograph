# Optional type-aware resolution

Lithograph’s baseline graph always runs without compiler services. Optional
type-aware resolution is an explicit post-processing pass that upgrades only
unique `TypeRefs` and `UsesType` targets already present in the graph.

It supports Python, Rust, TypeScript, JavaScript, Java, C#, Go, C, and C++ at
the `UniqueName` capability level. It does not perform runtime receiver,
classpath/assembly, generic/template, overload, or compiler-program inference.
Ambiguous names remain unresolved. Disabling the pass makes no graph changes.
