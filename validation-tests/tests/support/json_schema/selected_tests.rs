//! The same JSON Schema cases exercised against one integration target's backend.

use crate::JSON_SCHEMA_TARGET;
use crate::support::json_schema::{conformance, corpus, probes};

#[test]
fn json_schema_conformance_cases_agree_across_targets() {
    conformance::run(Some(JSON_SCHEMA_TARGET));
}

#[test]
fn pattern_corpus_matches_identically_in_every_runtime() {
    corpus::pattern_corpus_matches_identically_in_every_runtime(Some(JSON_SCHEMA_TARGET));
}

#[test]
fn format_corpora_hold_in_every_runtime() {
    corpus::format_corpora_hold_in_every_runtime(Some(JSON_SCHEMA_TARGET));
}

#[test]
fn generated_output_compiles_for_every_probe_schema() {
    probes::run(Some(JSON_SCHEMA_TARGET));
}
