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
    /// None leaves language support unrestricted.
    languages: Option<&'static [Language]>,
    system_nexus: bool,
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
            languages: None,
            system_nexus: false,
        },
        Annotation {
            name: "namespace",
            sites: &[Interface],
            keys: &["value", "import"],
            language_keys: &["", "import"],
            repeatable: false,
            languages: Some(&[Language::Dotnet, Language::Go]),
            system_nexus: false,
        },
        Annotation {
            name: "facade",
            sites: &[Interface],
            keys: &["value"],
            language_keys: &[""],
            repeatable: false,
            languages: Some(&[Language::Go]),
            system_nexus: false,
        },
        Annotation {
            name: "operations-class",
            sites: &[Interface],
            keys: &["value", "import"],
            language_keys: &["", "import"],
            repeatable: false,
            languages: Some(&[Language::Dotnet]),
            system_nexus: true,
        },
        Annotation {
            name: "service-name",
            sites: &[Interface],
            keys: &["value", "name"],
            language_keys: &[],
            repeatable: false,
            languages: None,
            system_nexus: false,
        },
        Annotation {
            name: "delay-load-temporalio-workflow",
            sites: &[Interface],
            keys: &[],
            language_keys: &[],
            repeatable: false,
            languages: None,
            system_nexus: true,
        },
        Annotation {
            name: "proto",
            sites: &[Record, Enum, Flags, Variant, Alias, Resource],
            keys: &["value", "import"],
            language_keys: &["type", "import"],
            repeatable: false,
            languages: None,
            system_nexus: false,
        },
        Annotation {
            name: "type-parameter",
            sites: &[Alias, Resource],
            keys: &[],
            language_keys: &[],
            // A resource names each of its type parameters.
            repeatable: true,
            languages: None,
            system_nexus: false,
        },
        Annotation {
            name: "proto-field",
            sites: &[Field],
            keys: &["value"],
            language_keys: &[],
            repeatable: false,
            languages: None,
            system_nexus: false,
        },
        Annotation {
            name: "name",
            sites: &[Field],
            keys: &[],
            language_keys: &[""],
            repeatable: false,
            languages: None,
            system_nexus: false,
        },
        Annotation {
            name: "type",
            sites: &[Field, Alias, Resource],
            keys: &["value", "import"],
            language_keys: &["", "import", "from", "to"],
            repeatable: false,
            languages: None,
            system_nexus: false,
        },
        Annotation {
            name: "omit",
            sites: &[Field],
            keys: &[],
            language_keys: &[],
            repeatable: false,
            languages: None,
            system_nexus: false,
        },
        Annotation {
            name: "api-omit",
            sites: &[Field],
            keys: &[],
            language_keys: &[],
            repeatable: false,
            languages: None,
            system_nexus: false,
        },
        Annotation {
            name: "source",
            sites: &[Field],
            keys: &["value", "targets"],
            language_keys: &[""],
            repeatable: true,
            languages: None,
            system_nexus: false,
        },
        Annotation {
            name: "default",
            sites: &[Field],
            keys: &["value", "import", "targets"],
            language_keys: &["", "import"],
            repeatable: true,
            languages: None,
            system_nexus: false,
        },
        Annotation {
            name: "fallback",
            sites: &[EnumCase],
            keys: &[],
            language_keys: &[],
            repeatable: false,
            languages: None,
            system_nexus: false,
        },
        Annotation {
            name: "experimental",
            sites: &[Interface, Record, Variant, Function],
            keys: &[],
            language_keys: &[],
            repeatable: false,
            languages: None,
            system_nexus: false,
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
            languages: None,
            system_nexus: false,
        },
        Annotation {
            name: "operation",
            sites: &[Method, Function],
            keys: &["value", "name"],
            language_keys: &[],
            repeatable: false,
            languages: None,
            system_nexus: false,
        },
        Annotation {
            name: "poll",
            sites: &[Function],
            keys: &["value", "import", "targets", "items", "resume"],
            language_keys: &["", "import", "resume"],
            repeatable: false,
            languages: Some(&[Language::Python]),
            system_nexus: true,
        },
        Annotation {
            name: "serialization-context",
            sites: &[Function],
            keys: &["value", "import"],
            language_keys: &["", "import"],
            repeatable: false,
            languages: None,
            system_nexus: false,
        },
        Annotation {
            name: "output-transform",
            sites: &[Function],
            keys: &["value", "import"],
            language_keys: &["", "import", "type", "type-import"],
            repeatable: false,
            languages: None,
            system_nexus: false,
        },
        Annotation {
            name: "flatten-in-api",
            sites: &[Record],
            keys: &[],
            language_keys: &[],
            repeatable: false,
            languages: None,
            system_nexus: false,
        },
        Annotation {
            name: "flattened-type",
            sites: &[Field],
            keys: &["value", "import"],
            language_keys: &["", "import"],
            repeatable: false,
            languages: None,
            system_nexus: false,
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
            languages: None,
            system_nexus: false,
        },
        Annotation {
            name: "function-args",
            sites: &[Function],
            keys: &["varargs", "param", "typescript-drop-prefix"],
            language_keys: &[],
            repeatable: false,
            languages: None,
            system_nexus: false,
        },
        Annotation {
            name: "add-rpc-compatible-with",
            sites: &[Alias],
            keys: &["value"],
            language_keys: &[],
            repeatable: true,
            languages: None,
            system_nexus: false,
        },
    ]
};

impl Annotation {
    fn supports_language(&self, language: Language) -> bool {
        self.languages
            .is_none_or(|languages| languages.contains(&language))
    }

    fn argument_language(&self, key: &str) -> Option<Language> {
        all_languages().into_iter().find(|language| {
            self.language_keys.iter().any(|suffix| {
                if suffix.is_empty() {
                    key == language_key(*language)
                } else {
                    key == format!("{}-{suffix}", language_key(*language))
                }
            })
        })
    }

    fn applies_to(&self, directive: &Directive, language: Language) -> bool {
        // A default applies to every output language. Otherwise language-qualified
        // arguments scope the directive to their languages.
        directive.value("value").is_some()
            || !directive
                .args
                .keys()
                .any(|key| self.argument_language(key).is_some())
            || directive
                .args
                .keys()
                .any(|key| self.argument_language(key) == Some(language))
    }

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
        // An alias is the type parameter, and a resource names the alias of each one it has.
        if self.name == "type-parameter" {
            return site == Site::Resource && key == "value";
        }
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
        self.keys.contains(&key) || self.argument_language(key).is_some()
    }
}

#[derive(Clone, Copy)]
struct GenerationContext {
    language: Language,
    system_nexus: bool,
}

fn validate_docs(
    docs: Option<&str>,
    site: Site,
    path: &Path,
    context: &str,
    generation: Option<GenerationContext>,
) -> Result<()> {
    let directives = parse_directives(docs, path, context)?;
    let proto_backed = directives
        .iter()
        .any(|directive| directive.name() == "proto");
    // An alias is one type parameter, so only a resource repeats the directive.
    if site == Site::Alias
        && directives
            .iter()
            .filter(|directive| directive.name() == "type-parameter")
            .count()
            > 1
    {
        return Err(Error::InvalidWitDirective {
            path: path.to_path_buf(),
            context: context.to_string(),
            directive: "@nexus.type-parameter".to_string(),
            reason: "duplicate directive".to_string(),
        });
    }
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
                "type-parameter" => {
                    "is only supported on WIT type aliases and resources".to_string()
                }
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
        if let Some(generation) = generation
            && rule.applies_to(&directive, generation.language)
        {
            if !rule.supports_language(generation.language) {
                return Err(fail(format!(
                    "is not supported for {}; valid languages: {}",
                    language_key(generation.language),
                    rule.languages
                        .unwrap()
                        .iter()
                        .map(|language| language_key(*language))
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
            if rule.system_nexus && !generation.system_nexus {
                return Err(Error::SystemNexusRequired {
                    feature: rule.name,
                    context: context.to_string(),
                });
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
            if let Some(language) = rule.argument_language(key)
                && !rule.supports_language(language)
            {
                return Err(fail(format!(
                    "argument `{key}` is not supported for {}",
                    language_key(language)
                )));
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
    language: Option<Language>,
) -> Result<()> {
    let generation = language.map(|language| GenerationContext {
        language,
        system_nexus: crate::nexgen_config::current().system_nexus,
    });
    let validate_docs = |docs: Option<&str>, site: Site, path: &Path, context: &str| {
        validate_docs(docs, site, path, context, generation)
    };
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
            if let Some(languages) = rule.languages {
                assert!(!languages.is_empty());
                let unique: BTreeSet<_> = languages.iter().collect();
                assert_eq!(unique.len(), languages.len());
            }
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
    fn language_scoping_distinguishes_defaults_from_overrides() {
        for (input, applies_to_go, applies_to_dotnet) in [
            ("@nexus.operations-class \"Operations\"", true, true),
            ("@nexus.operations-class dotnet=\"Operations\"", false, true),
            (
                "@nexus.operations-class \"Default\" dotnet=\"Operations\"",
                true,
                true,
            ),
        ] {
            let directives =
                parse_directives(Some(input), Path::new("test.wit"), "interface").unwrap();
            let directive = &directives[0];
            let rule = directive.rule;
            assert_eq!(rule.applies_to(directive, Language::Go), applies_to_go);
            assert_eq!(
                rule.applies_to(directive, Language::Dotnet),
                applies_to_dotnet
            );
            for language in all_languages() {
                assert_eq!(
                    rule.supports_language(language),
                    language == Language::Dotnet
                );
            }
            assert!(rule.system_nexus);
        }
    }

    #[test]
    fn inactive_override_does_not_require_system_nexus() {
        validate_docs(
            Some("@nexus.operations-class dotnet=\"Operations\""),
            Site::Interface,
            Path::new("test.wit"),
            "interface",
            Some(GenerationContext {
                language: Language::Go,
                system_nexus: false,
            }),
        )
        .unwrap();
    }

    #[test]
    fn supported_override_is_valid_in_system_nexus() {
        validate_docs(
            Some("@nexus.operations-class dotnet=\"Operations\""),
            Site::Interface,
            Path::new("test.wit"),
            "interface",
            Some(GenerationContext {
                language: Language::Dotnet,
                system_nexus: true,
            }),
        )
        .unwrap();
    }

    #[test]
    fn guide_availability_index_matches_catalog() {
        let guide = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/GUIDE.md"));
        let index = guide
            .split_once("<!-- annotation-availability:start -->")
            .unwrap()
            .1
            .split_once("<!-- annotation-availability:end -->")
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
            .filter(|rule| rule.languages.is_some() || rule.system_nexus)
            .map(|rule| {
                vec![
                    format!("`@nexus.{}`", rule.name),
                    rule.languages.map_or_else(
                        || "none".to_string(),
                        |languages| {
                            languages
                                .iter()
                                .map(|language| language_key(*language))
                                .collect::<Vec<_>>()
                                .join(", ")
                        },
                    ),
                    if rule.system_nexus {
                        "required"
                    } else {
                        "not required by catalog"
                    }
                    .to_string(),
                ]
            })
            .collect();
        assert_eq!(rows, expected);
    }

    #[test]
    fn only_a_resource_repeats_type_parameter() {
        let path = Path::new("test.wit");
        let resource = "@nexus.type-parameter \"key-t\"\n@nexus.type-parameter \"value-t\"";
        assert!(validate_docs(Some(resource), Site::Resource, path, "resource", None).is_ok());
        let alias = "@nexus.type-parameter\n@nexus.type-parameter";
        let err = validate_docs(Some(alias), Site::Alias, path, "type alias", None).unwrap_err();
        assert!(err.to_string().contains("duplicate directive"), "{err}");
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
