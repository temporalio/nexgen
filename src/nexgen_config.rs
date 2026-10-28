//! Scoped generation settings shared across the compiler.
//!
//! Configuration is thread-local so concurrent in-process callers cannot affect
//! one another. Scopes nest and restore the prior configuration during unwinding.

use std::cell::RefCell;

use crate::generator::GenerationMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NexgenConfig {
    pub mode: GenerationMode,
    pub system_nexus: bool,
    pub targets: GenerationTargets,
}

/// The code that calls the generated API. The models are the same for each target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenerationTargets {
    /// Workflow code.
    pub workflow: bool,
    /// Code outside a Workflow, through a client class that inherits a generated extension.
    pub client: bool,
}

/// One generation target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenerationTarget {
    Workflow,
    Client,
}

impl GenerationTarget {
    /// The directory of the target's package inside the output directory of a service. The
    /// Workflow target writes into that directory itself. Each language applies its own casing.
    pub fn package_dir(self) -> Option<&'static str> {
        match self {
            Self::Workflow => None,
            Self::Client => Some("client"),
        }
    }
}

impl Default for GenerationTargets {
    fn default() -> Self {
        Self {
            workflow: true,
            client: false,
        }
    }
}

impl Default for NexgenConfig {
    fn default() -> Self {
        Self {
            mode: GenerationMode::DefinitionsOnly,
            system_nexus: false,
            targets: GenerationTargets::default(),
        }
    }
}

thread_local! {
    static CONFIGS: RefCell<Vec<NexgenConfig>> = RefCell::new(Vec::new());
}

pub fn current() -> NexgenConfig {
    CONFIGS.with(|configs| configs.borrow().last().copied().unwrap_or_default())
}

pub fn scope(config: NexgenConfig) -> NexgenConfigScope {
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
    use super::{GenerationTargets, NexgenConfig, current, scope};
    use crate::generator::GenerationMode;

    #[test]
    fn restores_nested_configuration_after_unwinding() {
        let outer = NexgenConfig {
            mode: GenerationMode::DefinitionsOnly,
            system_nexus: true,
            targets: GenerationTargets::default(),
        };
        {
            let _outer_scope = scope(outer);
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
            mode: GenerationMode::DefinitionsOnly,
            system_nexus: true,
            targets: GenerationTargets::default(),
        };
        {
            let _scope = scope(config);
            assert_eq!(current(), config);
            assert_eq!(
                std::thread::spawn(current).join().unwrap(),
                NexgenConfig::default()
            );
        }
    }
}
