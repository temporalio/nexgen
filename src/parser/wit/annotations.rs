//! The WIT annotation vocabulary. Validate the entire resolved input before
//! lowering, not just the declarations that happen to be used by a target.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Site {
    Package,
    World,
    WorldItem,
    Interface,
    Record,
    Field,
    Enum,
    EnumCase,
    Flags,
    Flag,
    Variant,
    VariantCase,
    Alias,
    Resource,
    Constructor,
    Method,
    Function,
}

impl Site {
    fn label(self) -> &'static str {
        match self {
            Self::Package => "package",
            Self::World => "world",
            Self::WorldItem => "world import/export",
            Self::Interface => "interface",
            Self::Record => "record",
            Self::Field => "record field",
            Self::Enum => "enum",
            Self::EnumCase => "enum case",
            Self::Flags => "flags",
            Self::Flag => "flag",
            Self::Variant => "variant",
            Self::VariantCase => "variant case",
            Self::Alias => "type alias",
            Self::Resource => "resource",
            Self::Constructor => "constructor",
            Self::Method => "resource method",
            Self::Function => "freestanding function",
        }
    }
}

#[derive(Debug)]
pub(super) struct Annotation {
    pub(super) name: &'static str,
    sites: &'static [Site],
    /// Literal argument names, including `value` for the positional argument.
    keys: &'static [&'static str],
    /// Language-prefixed arguments: "" means a bare language name.
    language_keys: &'static [&'static str],
    repeatable: bool,
}

// Keep each rule here, rather than maintaining per-builder allowlists.
const CATALOG: &[Annotation] = {
    use Site::*;
    &[
        Annotation {
            name: "endpoint",
            sites: &[Interface],
            keys: &["value"],
            language_keys: &[""],
            repeatable: false,
        },
        Annotation {
            name: "namespace",
            sites: &[Interface],
            keys: &["value", "import"],
            language_keys: &["", "import"],
            repeatable: false,
        },
        Annotation {
            name: "operations-class",
            sites: &[Interface],
            keys: &["value", "import"],
            language_keys: &["", "import"],
            repeatable: false,
        },
        Annotation {
            name: "service-name",
            sites: &[Interface],
            keys: &["value", "name"],
            language_keys: &[],
            repeatable: false,
        },
        Annotation {
            name: "delay-load-temporalio-workflow",
            sites: &[Interface],
            keys: &[],
            language_keys: &[],
            repeatable: false,
        },
        Annotation {
            name: "proto",
            sites: &[Record, Enum, Flags, Variant, Alias, Resource],
            keys: &["value", "import"],
            language_keys: &["type", "import"],
            repeatable: false,
        },
        Annotation {
            name: "type-parameter",
            sites: &[Alias],
            keys: &[],
            language_keys: &[],
            repeatable: false,
        },
        Annotation {
            name: "proto-field",
            sites: &[Field],
            keys: &["value"],
            language_keys: &[],
            repeatable: false,
        },
        Annotation {
            name: "name",
            sites: &[Field],
            keys: &[],
            language_keys: &[""],
            repeatable: false,
        },
        Annotation {
            name: "type",
            sites: &[Field, Alias, Resource],
            keys: &["value", "import"],
            language_keys: &["", "import", "from", "to"],
            repeatable: false,
        },
        Annotation {
            name: "omit",
            sites: &[Field],
            keys: &[],
            language_keys: &[],
            repeatable: false,
        },
        Annotation {
            name: "api-omit",
            sites: &[Field],
            keys: &[],
            language_keys: &[],
            repeatable: false,
        },
        Annotation {
            name: "source",
            sites: &[Field],
            keys: &["value", "targets"],
            language_keys: &[""],
            repeatable: true,
        },
        Annotation {
            name: "default",
            sites: &[Field],
            keys: &["value", "import", "targets"],
            language_keys: &["", "import"],
            repeatable: true,
        },
        Annotation {
            name: "fallback",
            sites: &[EnumCase],
            keys: &[],
            language_keys: &[],
            repeatable: false,
        },
        Annotation {
            name: "experimental",
            sites: &[Interface, Record, Variant, Function],
            keys: &[],
            language_keys: &[],
            repeatable: false,
        },
        Annotation {
            name: "doc",
            sites: &[
                Record,
                Field,
                Variant,
                VariantCase,
                Resource,
                Method,
                Function,
            ],
            keys: &["value", "import", "returns"],
            language_keys: &["", "import", "returns"],
            repeatable: false,
        },
        Annotation {
            name: "operation",
            sites: &[Method, Function],
            keys: &["value", "name"],
            language_keys: &[],
            repeatable: false,
        },
        Annotation {
            name: "poll",
            sites: &[Function],
            keys: &["value", "import", "targets", "items", "resume"],
            language_keys: &["", "import", "resume"],
            repeatable: false,
        },
        Annotation {
            name: "serialization-context",
            sites: &[Function],
            keys: &["value", "import"],
            language_keys: &["", "import"],
            repeatable: false,
        },
        Annotation {
            name: "output-transform",
            sites: &[Function],
            keys: &["value", "import"],
            language_keys: &["", "import", "type", "type-import"],
            repeatable: false,
        },
        Annotation {
            name: "flatten-in-api",
            sites: &[Record, Alias, Resource],
            keys: &[],
            language_keys: &[],
            repeatable: false,
        },
        Annotation {
            name: "flattened-type",
            sites: &[Field],
            keys: &["value", "import"],
            language_keys: &["", "import"],
            repeatable: false,
        },
        Annotation {
            name: "function",
            sites: &[Alias, Field],
            keys: &[
                "signature",
                "args-field",
                "primary",
                "alternate-type",
                "converter",
                "name-extractor",
                "call-extractor",
                "result-type-parameter",
                "value-type",
                "args-type",
                "result",
                "import",
            ],
            language_keys: &[
                "converter",
                "name-extractor",
                "call-extractor",
                "value-type",
                "args-type",
                "result",
                "import",
            ],
            repeatable: false,
        },
        Annotation {
            name: "function-args",
            sites: &[Function],
            keys: &["varargs", "param", "typescript-drop-prefix"],
            language_keys: &[],
            repeatable: false,
        },
        Annotation {
            name: "add-rpc-compatible-with",
            sites: &[Alias],
            keys: &["value"],
            language_keys: &[],
            repeatable: true,
        },
    ]
};

impl Annotation {
    pub(super) fn parse(name: &str, path: &Path, context: &str) -> Result<&'static Self> {
        CATALOG
            .iter()
            .find(|rule| rule.name == name)
            .ok_or_else(|| Error::InvalidWitDirective {
                path: path.to_path_buf(),
                context: context.to_string(),
                directive: format!("@nexus.{name}"),
                reason: "unknown annotation".to_string(),
            })
    }

    fn accepts_key(&self, key: &str, site: Site) -> bool {
        if self.name == "type"
            && site == Site::Field
            && (key.ends_with("-from") || key.ends_with("-to"))
        {
            return false;
        }
        // These options have different consumers at the two function sites.
        if self.name == "function"
            && ((site == Site::Field && key == "signature")
                || (site == Site::Alias && (key == "result" || key.ends_with("-result"))))
        {
            return false;
        }
        if self.name == "doc"
            && site != Site::Function
            && (key == "returns" || key.ends_with("-returns"))
        {
            return false;
        }
        self.keys.contains(&key)
            || all_languages().iter().any(|language| {
                self.language_keys.iter().any(|suffix| {
                    if suffix.is_empty() {
                        key == language_key(*language)
                    } else {
                        key == format!("{}-{suffix}", language_key(*language))
                    }
                })
            })
    }
}

fn validate_docs(docs: Option<&str>, site: Site, path: &Path, context: &str) -> Result<()> {
    let directives = parse_directives(docs, path, context)?;
    let proto_backed = directives
        .iter()
        .any(|directive| directive.name() == "proto");
    for directive in directives {
        let fail = |reason: String| Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: context.to_string(),
            directive: format!("@nexus.{}", directive.name()),
            reason,
        };
        let rule = directive.rule;
        if !rule.sites.contains(&site) {
            let reason = match rule.name {
                "type-parameter" => "is only supported on WIT type aliases".to_string(),
                "fallback" => "is only supported on enum cases".to_string(),
                "type"
                    if proto_backed
                        && matches!(
                            site,
                            Site::Record | Site::Enum | Site::Flags | Site::Variant
                        ) =>
                {
                    "cannot be combined with `@nexus.proto` on a native WIT declaration".to_string()
                }
                "omit" if site == Site::Record => {
                    "mark record fields with `@nexus.omit`; type-level omit is no longer supported"
                        .to_string()
                }
                _ => format!(
                    "is not supported on {}; valid placements: {}",
                    site.label(),
                    rule.sites
                        .iter()
                        .map(|site| site.label())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            };
            return Err(fail(reason));
        }
        for key in directive.args.keys() {
            if !rule.accepts_key(key, site) {
                return Err(fail(format!(
                    "unknown argument `{key}` on {}",
                    site.label()
                )));
            }
        }
    }
    Ok(())
}

/// Shared parsing rules apply to every consumer, including scaffolding commands.
/// Placement-specific argument restrictions are checked by `validate_docs`.
pub(super) fn validate_parsed(directives: &[Directive], path: &Path, context: &str) -> Result<()> {
    let mut seen = BTreeSet::new();
    for directive in directives {
        let fail = |reason: String| Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: context.to_string(),
            directive: format!("@nexus.{}", directive.name()),
            reason,
        };
        let rule = directive.rule;
        if !rule.repeatable && !seen.insert(rule.name) {
            return Err(fail("duplicate directive".to_string()));
        }
        for key in directive.args.keys() {
            if !rule.sites.iter().any(|site| rule.accepts_key(key, *site)) {
                let reason = if rule.keys.is_empty() && rule.language_keys.is_empty() {
                    "does not take arguments".to_string()
                } else if rule.name == "source" && key.ends_with("-support-import") {
                    format!(
                        "`{key}` is no longer supported; remove it and call a helper from `--support-package` in the source expression"
                    )
                } else {
                    format!("unknown argument `{key}`")
                };
                return Err(fail(reason));
            }
        }
    }
    Ok(())
}

pub(super) fn validate(
    resolve: &Resolve,
    primary: PackageId,
    origins: &PackageOrigins,
    fallback: &Path,
) -> Result<()> {
    let path_for = |package: Option<PackageId>| {
        package
            .filter(|id| *id != primary)
            .and_then(|id| origins.get(&id))
            .map_or(fallback, PathBuf::as_path)
    };
    for (id, package) in resolve.packages.iter() {
        validate_docs(
            package.docs.contents.as_deref(),
            Site::Package,
            path_for(Some(id)),
            &format!("package `{}`", package.name),
        )?;
    }
    for (_, world) in resolve.worlds.iter() {
        let path = path_for(world.package);
        let context = format!("world `{}`", world.name);
        validate_docs(world.docs.contents.as_deref(), Site::World, path, &context)?;
        for item in world.imports.values().chain(world.exports.values()) {
            match item {
                WorldItem::Interface { docs, .. } => {
                    validate_docs(docs.contents.as_deref(), Site::WorldItem, path, &context)?
                }
                WorldItem::Function(function) => validate_docs(
                    function.docs.contents.as_deref(),
                    Site::WorldItem,
                    path,
                    &format!("{context} function `{}`", function.name),
                )?,
                WorldItem::Type { .. } => {}
            }
        }
    }
    for (_, interface) in resolve.interfaces.iter() {
        let path = path_for(interface.package);
        let context = format!(
            "interface `{}`",
            interface.name.as_deref().unwrap_or("unnamed-interface")
        );
        validate_docs(
            interface.docs.contents.as_deref(),
            Site::Interface,
            path,
            &context,
        )?;
        for function in interface.functions.values() {
            let site = match function.kind {
                FunctionKind::Constructor(_) => Site::Constructor,
                FunctionKind::Method(_) | FunctionKind::AsyncMethod(_) => Site::Method,
                FunctionKind::Static(_) | FunctionKind::AsyncStatic(_) => Site::Method,
                FunctionKind::Freestanding | FunctionKind::AsyncFreestanding => Site::Function,
            };
            validate_docs(
                function.docs.contents.as_deref(),
                site,
                path,
                &format!("{context} function `{}`", function.name),
            )?;
        }
    }
    for (id, ty) in resolve.types.iter() {
        let package = match ty.owner {
            TypeOwner::Interface(id) => resolve.interfaces[id].package,
            TypeOwner::World(id) => resolve.worlds[id].package,
            TypeOwner::None => None,
        };
        let path = path_for(package);
        let context = format!("type `{}`", wit_type_full_name(resolve, id));
        let site = match ty.kind {
            TypeDefKind::Record(_) => Site::Record,
            TypeDefKind::Enum(_) => Site::Enum,
            TypeDefKind::Flags(_) => Site::Flags,
            TypeDefKind::Variant(_) => Site::Variant,
            TypeDefKind::Resource => Site::Resource,
            _ => Site::Alias,
        };
        validate_docs(ty.docs.contents.as_deref(), site, path, &context)?;
        match &ty.kind {
            TypeDefKind::Record(record) => {
                for field in &record.fields {
                    validate_docs(
                        field.docs.contents.as_deref(),
                        Site::Field,
                        path,
                        &format!("{context} field `{}`", field.name),
                    )?;
                }
            }
            TypeDefKind::Enum(enumeration) => {
                for case in &enumeration.cases {
                    validate_docs(
                        case.docs.contents.as_deref(),
                        Site::EnumCase,
                        path,
                        &format!("{context} case `{}`", case.name),
                    )?;
                }
            }
            TypeDefKind::Variant(variant) => {
                for case in &variant.cases {
                    validate_docs(
                        case.docs.contents.as_deref(),
                        Site::VariantCase,
                        path,
                        &format!("{context} case `{}`", case.name),
                    )?;
                }
            }
            TypeDefKind::Flags(flags) => {
                for flag in &flags.flags {
                    validate_docs(
                        flag.docs.contents.as_deref(),
                        Site::Flag,
                        path,
                        &format!("{context} case `{}`", flag.name),
                    )?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_unambiguous() {
        let mut names = BTreeSet::new();
        for rule in CATALOG {
            assert!(
                names.insert(rule.name),
                "duplicate annotation {}",
                rule.name
            );
            assert!(!rule.sites.is_empty());
            let mut sites = Vec::new();
            for site in rule.sites {
                assert!(!sites.contains(site));
                sites.push(*site);
            }
        }
    }

    #[test]
    fn guide_placement_index_matches_catalog() {
        let guide = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/GUIDE.md"));
        let index = guide
            .split_once("<!-- annotation-placements:start -->")
            .unwrap()
            .1
            .split_once("<!-- annotation-placements:end -->")
            .unwrap()
            .0;
        let rows: Vec<Vec<&str>> = index
            .lines()
            .filter(|line| line.trim_start().starts_with("| `@nexus."))
            .map(|line| {
                line.trim()
                    .trim_matches('|')
                    .split('|')
                    .map(str::trim)
                    .collect()
            })
            .collect();
        let expected: Vec<Vec<String>> = CATALOG
            .iter()
            .map(|rule| {
                vec![
                    format!("`@nexus.{}`", rule.name),
                    rule.sites
                        .iter()
                        .map(|site| site.label())
                        .collect::<Vec<_>>()
                        .join(", "),
                ]
            })
            .collect();
        assert_eq!(rows, expected);
    }

    #[test]
    fn argument_families_are_site_specific() {
        let rule = |name| CATALOG.iter().find(|rule| rule.name == name).unwrap();
        for language in all_languages() {
            let prefix = language_key(language);
            let conversion = format!("{prefix}-from");
            assert!(rule("type").accepts_key(&conversion, Site::Alias));
            assert!(!rule("type").accepts_key(&conversion, Site::Field));
            assert!(rule("name").accepts_key(prefix, Site::Field));
            assert!(!rule("name").accepts_key(&conversion, Site::Field));
            assert!(rule("proto").accepts_key(&format!("{prefix}-type"), Site::Record));
        }
        assert!(rule("function").accepts_key("signature", Site::Alias));
        assert!(!rule("function").accepts_key("signature", Site::Field));
        assert!(rule("function").accepts_key("result", Site::Field));
        assert!(!rule("function").accepts_key("result", Site::Alias));
        assert!(rule("doc").accepts_key("returns", Site::Function));
        assert!(!rule("doc").accepts_key("returns", Site::Field));
    }

    #[test]
    fn repeatable_annotations_preserve_each_value() {
        let directives = parse_directives(
            Some("@nexus.add-rpc-compatible-with \"first\"\n@nexus.add-rpc-compatible-with \"second\""),
            Path::new("test.wit"),
            "type alias",
        ).unwrap();
        let values: Vec<_> = directives
            .iter()
            .map(|directive| directive.value("value"))
            .collect();
        assert_eq!(values, [Some("first"), Some("second")]);
    }
}
