//! Cross-target JSON Schema validation with real language toolchains.

mod support;

use support::json_schema::{conformance, corpus, probes};

#[test]
fn manifest_runtime() {
    conformance::run(None);
}

#[test]
fn pattern_corpus_matches_identically_in_every_runtime() {
    corpus::pattern_corpus_matches_identically_in_every_runtime(None);
}

#[test]
fn format_corpora_hold_in_every_runtime() {
    corpus::format_corpora_hold_in_every_runtime(None);
}

#[test]
fn probe_matrix() {
    probes::run(None);
}
