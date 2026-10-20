#![no_main]
//! A project's `graph.json` from disk: deserialise and validate (types, cycles, required
//! inputs, unknown kinds) without panicking; evaluation is not attempted.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(graph) = serde_json::from_slice::<jd_graph::Graph>(data) {
        let registry = jd_graph::Registry::with_builtins();
        let cache = jd_graph::Cache::with_capacity(0);
        let _ = jd_graph::Evaluator::new(&registry, &cache).validate(&graph);
    }
});
