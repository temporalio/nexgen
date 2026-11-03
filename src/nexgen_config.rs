//! Scoped generation settings shared across the compiler.
//!
//! Configuration is thread-local so concurrent in-process callers cannot affect
//! one another. Scopes nest and restore the prior configuration during unwinding.

use std::cell::RefCell;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NexgenConfig {
    pub system_nexus: bool,
    pub targets: Vec<GenerationTarget>,
}

/// One generation target.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Deserialize, clap::ValueEnum,
)]
#[serde(rename_all = "lowercase")]
pub enum GenerationTarget {
    /// Workflow code.
    Workflow,
    /// Code outside a Workflow, through a client class that inherits a generated extension.
    Client,
}

thread_local! {
    static CONFIGS: RefCell<Vec<NexgenConfig>> = RefCell::new(Vec::new());
}

pub fn current() -> NexgenConfig {
    CONFIGS.with(|configs| configs.borrow().last().cloned().unwrap_or_default())
}

pub fn scope(mut config: NexgenConfig) -> NexgenConfigScope {
    config.targets.sort_unstable();
    config.targets.dedup();
    CONFIGS.with(|configs| configs.borrow_mut().push(config));
    NexgenConfigScope
}

pub struct NexgenConfigScope;

impl Drop for NexgenConfigScope {
    fn drop(&mut self) {
        CONFIGS.with(|configs| {
            configs
                .borrow_mut()
                .pop()
                .expect("NexGen configuration scope must be present");
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{GenerationTarget, NexgenConfig, current, scope};

    #[test]
    fn normalizes_target_order_and_duplicates_at_scope_entry() {
        let _scope = scope(NexgenConfig {
            targets: vec![
                GenerationTarget::Client,
                GenerationTarget::Workflow,
                GenerationTarget::Client,
                GenerationTarget::Workflow,
            ],
            ..Default::default()
        });
        assert_eq!(
            current().targets,
            vec![GenerationTarget::Workflow, GenerationTarget::Client]
        );
    }

    #[test]
    fn restores_nested_configuration_after_unwinding() {
        let outer = NexgenConfig {
            system_nexus: true,
            targets: Vec::new(),
        };
        {
            let _outer_scope = scope(outer.clone());
            assert_eq!(current(), outer);
            let result = std::panic::catch_unwind(|| {
                let _inner_scope = scope(NexgenConfig::default());
                panic!("test unwind");
            });
            assert!(result.is_err());
            assert_eq!(current(), outer);
        }
        assert_eq!(current(), NexgenConfig::default());
    }

    #[test]
    fn is_isolated_per_thread() {
        let config = NexgenConfig {
            system_nexus: true,
            targets: Vec::new(),
        };
        {
            let _scope = scope(config.clone());
            assert_eq!(current(), config);
            assert_eq!(
                std::thread::spawn(current).join().unwrap(),
                NexgenConfig::default()
            );
        }
    }
}
